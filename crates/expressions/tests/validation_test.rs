use std::collections::HashMap;

use spin_expressions::{provider::ProviderVariableKind, Key, Provider, ProviderResolver};
use spin_locked_app::Variable;

#[derive(Default)]
struct ResolverTester {
    providers: Vec<Box<dyn Provider>>,
    variables: HashMap<String, Variable>,
}

impl ResolverTester {
    fn new() -> Self {
        Self::default()
    }

    fn with_dynamic_provider(mut self) -> Self {
        self.providers.push(Box::new(DynamicProvider));
        self
    }

    fn with_static_provider(mut self, key: &str, value: Option<&str>) -> Self {
        self.providers
            .push(Box::new(StaticProvider::with_variable(key, value)));
        self
    }

    fn with_variable(mut self, (key, default): (&str, Option<&str>)) -> Self {
        self.variables.insert(
            key.to_string(),
            Variable {
                description: None,
                default: default.map(ToString::to_string),
                secret: false,
            },
        );
        self
    }

    fn make_resolver(self) -> anyhow::Result<ProviderResolver> {
        let mut provider_resolver = ProviderResolver::new(self.variables)?;

        for provider in self.providers {
            provider_resolver.add_provider(provider as _);
        }

        Ok(provider_resolver)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn single_static_provider_with_no_variable_provided_is_valid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_static_provider("foo", Some("bar"))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_single_static_provider_has_variable_value_validation_succeeds() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_static_provider("foo", Some("bar"))
        .with_variable(("foo", None))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_a_single_static_provider_and_it_does_not_contain_a_required_variable_then_validation_fails(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_static_provider("foo", Some("bar"))
        .with_variable(("bar", None))
        .make_resolver()?;

    assert!(resolver.validate_variables().await.is_err());

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_a_dynamic_provider_then_validation_succeeds_even_if_a_static_provider_without_the_variable_is_in_play(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_dynamic_provider()
        .with_variable(("bar", None))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_a_dynamic_provider_and_a_static_provider_then_validation_succeeds_even_if_a_static_provider_without_the_variable_is_in_play(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_dynamic_provider()
        .with_static_provider("foo", Some("bar"))
        .with_variable(("baz", None))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_a_dynamic_provider_and_a_static_provider_then_validation_succeeds_even_if_a_static_provider_with_the_variable_is_in_play(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_dynamic_provider()
        .with_static_provider("foo", Some("bar"))
        .with_variable(("baz", Some("coo")))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_two_static_providers_where_one_has_data_is_valid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_static_provider("foo", Some("bar"))
        .with_static_provider("baz", Some("hay"))
        .with_variable(("foo", None))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_two_static_providers_where_first_provider_does_not_have_data_while_second_provider_does(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_static_provider("foo", Some("bar"))
        .with_static_provider("baz", Some("hay"))
        .with_variable(("baz", None))
        .make_resolver()?;

    resolver.validate_variables().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn if_there_is_two_static_providers_neither_having_data_is_invalid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_static_provider("foo", Some("bar"))
        .with_static_provider("baz", Some("hay"))
        .with_variable(("hello", None))
        .make_resolver()?;

    assert!(resolver.validate_variables().await.is_err());

    Ok(())
}

#[derive(Debug)]
struct StaticProvider {
    variables: HashMap<String, Option<String>>,
}

impl StaticProvider {
    fn with_variable(key: &str, value: Option<&str>) -> Self {
        Self {
            variables: HashMap::from([(key.into(), value.map(|v| v.into()))]),
        }
    }
}

#[spin_world::async_trait]
impl Provider for StaticProvider {
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        Ok(self.variables.get(key.as_str()).cloned().flatten())
    }

    fn kind(&self) -> ProviderVariableKind {
        ProviderVariableKind::Static
    }
}

#[derive(Debug)]
struct DynamicProvider;

#[spin_world::async_trait]
impl Provider for DynamicProvider {
    async fn get(&self, _key: &Key) -> anyhow::Result<Option<String>> {
        panic!("validation should never call get for a dynamic provider")
    }

    fn kind(&self) -> ProviderVariableKind {
        ProviderVariableKind::Dynamic
    }
}
