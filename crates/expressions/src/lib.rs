pub mod provider;
mod template;

use std::{borrow::Cow, collections::HashMap, fmt::Debug};

use spin_locked_app::Variable;

pub use async_trait;

pub use provider::Provider;
use template::Part;
pub use template::Template;

/// A [`ProviderResolver`] that can be shared.
pub type SharedPreparedResolver =
    std::sync::Arc<std::sync::OnceLock<std::sync::Arc<PreparedResolver>>>;

/// A [`Resolver`] which is extended by [`Provider`]s.
#[derive(Debug, Default)]
pub struct ProviderResolver {
    internal: Resolver,
    providers: Vec<Box<dyn Provider>>,
}

impl ProviderResolver {
    /// Creates a Resolver for the given Tree.
    pub fn new(variables: impl IntoIterator<Item = (String, Variable)>) -> Result<Self> {
        Ok(Self {
            internal: Resolver::new(variables)?,
            providers: Default::default(),
        })
    }

    /// Adds component variable values to the Resolver.
    pub fn add_component_variables(
        &mut self,
        component_id: impl Into<String>,
        variables: impl IntoIterator<Item = (String, String)>,
    ) -> Result<()> {
        self.internal
            .add_component_variables(component_id, variables)
    }

    /// Adds a variable Provider to the Resolver.
    pub fn add_provider(&mut self, provider: Box<dyn Provider>) {
        self.providers.push(provider);
    }

    /// Resolves a variable value for the given path.
    pub async fn resolve(&self, component_id: &str, key: Key<'_>) -> Result<String> {
        let template = self.internal.get_template(component_id, key)?;
        self.resolve_template(template).await
    }

    /// Resolves all variables for the given component.
    pub async fn resolve_all(&self, component_id: &str) -> Result<Vec<(String, String)>> {
        use futures::FutureExt;

        let Some(keys2templates) = self.internal.component_configs.get(component_id) else {
            return Ok(vec![]);
        };

        let resolve_futs = keys2templates.iter().map(|(key, template)| {
            self.resolve_template(template)
                .map(|r| r.map(|value| (key.to_string(), value)))
        });

        futures::future::try_join_all(resolve_futs).await
    }

    /// Resolves the given template.
    pub async fn resolve_template(&self, template: &Template) -> Result<String> {
        let mut resolved_parts: Vec<Cow<str>> = Vec::with_capacity(template.parts().len());
        for part in template.parts() {
            resolved_parts.push(match part {
                Part::Lit(lit) => lit.as_ref().into(),
                Part::Expr(var) => self.resolve_variable(var).await?.into(),
            });
        }
        Ok(resolved_parts.concat())
    }

    /// Fully resolve all variables into a [`PreparedResolver`].
    pub async fn prepare(&self) -> Result<PreparedResolver> {
        let mut variables = HashMap::new();
        for name in self.internal.variables.keys() {
            let value = self.resolve_variable(name).await?;
            variables.insert(name.clone(), value);
        }
        Ok(PreparedResolver { variables })
    }

    async fn resolve_variable(&self, key: &str) -> Result<String> {
        for provider in &self.providers {
            if let Some(value) = provider.get(&Key(key)).await.map_err(Error::Provider)? {
                return Ok(value);
            }
        }
        self.internal.resolve_variable(key)
    }
}

/// A variable resolver.
#[derive(Debug, Default)]
pub struct Resolver {
    // variable key -> variable
    variables: HashMap<String, Variable>,
    // component ID -> variable key -> variable value template
    component_configs: HashMap<String, HashMap<String, Template>>,
}

impl Resolver {
    /// Creates a Resolver for the given Tree.
    pub fn new(variables: impl IntoIterator<Item = (String, Variable)>) -> Result<Self> {
        let variables: HashMap<_, _> = variables.into_iter().collect();
        // Validate keys so that we can rely on them during resolution
        variables.keys().try_for_each(|key| Key::validate(key))?;
        Ok(Self {
            variables,
            component_configs: Default::default(),
        })
    }

    /// Adds component variable values to the Resolver.
    pub fn add_component_variables(
        &mut self,
        component_id: impl Into<String>,
        variables: impl IntoIterator<Item = (String, String)>,
    ) -> Result<()> {
        let component_id = component_id.into();
        let templates = variables
            .into_iter()
            .map(|(key, val)| {
                // Validate variable keys so that we can rely on them during resolution
                Key::validate(&key)?;
                let template = self.validate_template(val)?;
                Ok((key, template))
            })
            .collect::<Result<_>>()?;

        self.component_configs.insert(component_id, templates);

        Ok(())
    }

    /// Resolves a variable value for the given path.
    pub fn resolve(&self, component_id: &str, key: Key<'_>) -> Result<String> {
        let template = self.get_template(component_id, key)?;
        self.resolve_template(template)
    }

    /// Resolves the given template.
    fn resolve_template(&self, template: &Template) -> Result<String> {
        let mut resolved_parts: Vec<Cow<str>> = Vec::with_capacity(template.parts().len());
        for part in template.parts() {
            resolved_parts.push(match part {
                Part::Lit(lit) => lit.as_ref().into(),
                Part::Expr(var) => self.resolve_variable(var)?.into(),
            });
        }
        Ok(resolved_parts.concat())
    }

    /// Gets a template for the given path.
    fn get_template(&self, component_id: &str, key: Key<'_>) -> Result<&Template> {
        let configs = self.component_configs.get(component_id).ok_or_else(|| {
            Error::Undefined(format!("no variable for component {component_id:?}"))
        })?;
        let key = key.as_ref();
        let template = configs
            .get(key)
            .ok_or_else(|| Error::Undefined(format!("no variable for {component_id:?}.{key:?}")))?;
        Ok(template)
    }

    fn resolve_variable(&self, key: &str) -> Result<String> {
        let var = self
            .variables
            .get(key)
            // This should have been caught by validate_template
            .ok_or_else(|| Error::InvalidName(key.to_string()))?;

        var.default.clone().ok_or_else(|| {
            Error::Provider(anyhow::anyhow!(
                "no provider resolved required variable {key:?}"
            ))
        })
    }

    fn validate_template(&self, template: String) -> Result<Template> {
        let template = Template::new(template)?;
        // Validate template variables are valid
        template.parts().try_for_each(|part| match part {
            Part::Expr(var) if !self.variables.contains_key(var.as_ref()) => {
                Err(Error::InvalidTemplate(format!("unknown variable {var:?}")))
            }
            _ => Ok(()),
        })?;
        Ok(template)
    }
}

/// A resolver who has resolved all variables.
#[derive(Default)]
pub struct PreparedResolver {
    variables: HashMap<String, String>,
}

impl PreparedResolver {
    /// Resolves a the given template.
    pub fn resolve_template(&self, template: &Template) -> Result<String> {
        let mut resolved_parts: Vec<Cow<str>> = Vec::with_capacity(template.parts().len());
        for part in template.parts() {
            resolved_parts.push(match part {
                Part::Lit(lit) => lit.as_ref().into(),
                Part::Expr(var) => self.resolve_variable(var)?.into(),
            });
        }
        Ok(resolved_parts.concat())
    }

    fn resolve_variable(&self, key: &str) -> Result<String> {
        self.variables
            .get(key)
            .cloned()
            .ok_or(Error::InvalidName(key.to_string()))
    }
}

/// A variable key
#[derive(Debug, PartialEq, Eq)]
pub struct Key<'a>(&'a str);

impl<'a> Key<'a> {
    /// Creates a new Key.
    pub fn new(key: &'a str) -> Result<Self> {
        Self::validate(key)?;
        Ok(Self(key))
    }

    pub fn as_str(&self) -> &str {
        self.0
    }

    // To allow various (env var, file path) transformations:
    // - must start with an ASCII letter
    // - underscores are allowed; one at a time between other characters
    // - all other characters must be ASCII alphanumeric
    fn validate(key: &str) -> Result<()> {
        {
            if key.is_empty() {
                Err("must not be empty".to_string())
            } else if let Some(invalid) = key
                .chars()
                .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == &'_'))
            {
                Err(format!("invalid character {invalid:?}. Variable names may contain only lower-case letters, numbers, and underscores."))
            } else if !key.bytes().next().unwrap().is_ascii_lowercase() {
                Err("must start with a lowercase ASCII letter".to_string())
            } else if !key.bytes().last().unwrap().is_ascii_alphanumeric() {
                Err("must end with a lowercase ASCII letter or digit".to_string())
            } else if key.contains("__") {
                Err("must not contain multiple consecutive underscores".to_string())
            } else {
                Ok(())
            }
        }
        .map_err(|reason| Error::InvalidName(format!("{key:?}: {reason}")))
    }
}

impl<'a> TryFrom<&'a str> for Key<'a> {
    type Error = Error;

    fn try_from(value: &'a str) -> std::prelude::v1::Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for Key<'_> {
    fn as_ref(&self) -> &str {
        self.0
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A variable resolution error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Invalid variable name.
    #[error("invalid variable name: {0}")]
    InvalidName(String),

    /// Invalid variable template.
    #[error("invalid variable template: {0}")]
    InvalidTemplate(String),

    /// Variable provider error.
    #[error("provider error: {0:?}")]
    Provider(#[source] anyhow::Error),

    /// Undefined variable.
    #[error("undefined variable: {0}")]
    Undefined(String),
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    #[derive(Debug)]
    struct TestProvider;

    #[async_trait]
    impl Provider for TestProvider {
        async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
            match key.as_ref() {
                "required" => Ok(Some("provider-value".to_string())),
                "broken" => anyhow::bail!("broken"),
                _ => Ok(None),
            }
        }
    }

    async fn test_resolve(template: &str) -> Result<String> {
        let mut resolver = ProviderResolver::new([
            (
                "required".into(),
                Variable {
                    description: None,
                    default: None,
                    secret: false,
                },
            ),
            (
                "default".into(),
                Variable {
                    description: None,
                    default: Some("default-value".into()),
                    secret: false,
                },
            ),
        ])
        .unwrap();
        resolver
            .add_component_variables("test-component", [("test_key".into(), template.into())])
            .unwrap();
        resolver.add_provider(Box::new(TestProvider));
        resolver.resolve("test-component", Key("test_key")).await
    }

    #[tokio::test]
    async fn resolve_static() {
        assert_eq!(test_resolve("static-value").await.unwrap(), "static-value");
    }

    #[tokio::test]
    async fn resolve_variable_default() {
        assert_eq!(
            test_resolve("prefix-{{ default }}-suffix").await.unwrap(),
            "prefix-default-value-suffix"
        );
    }

    #[tokio::test]
    async fn resolve_variable_provider() {
        assert_eq!(
            test_resolve("prefix-{{ required }}-suffix").await.unwrap(),
            "prefix-provider-value-suffix"
        );
    }

    #[test]
    fn keys_good() {
        for key in ["a", "abc", "a1b2c3", "a_1", "a_1_b_3"] {
            Key::new(key).expect(key);
        }
    }

    #[test]
    fn keys_bad() {
        for key in ["", "aX", "1bc", "_x", "x.y", "x_", "a__b", "x-y"] {
            Key::new(key).expect_err(key);
        }
    }

    #[test]
    fn template_literal() {
        assert!(Template::new("hello").unwrap().is_literal());
        assert!(!Template::new("hello {{ world }}").unwrap().is_literal());
    }
}
