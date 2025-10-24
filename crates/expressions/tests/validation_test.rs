use std::collections::HashMap;

use async_trait::async_trait;
use spin_app::Variable;
use spin_expressions::{Key, Provider, ProviderResolver};

#[derive(Default)]
struct ResolverTester {
    providers: Vec<Box<dyn Provider>>,
    variables: HashMap<String, Variable>,
}

impl ResolverTester {
    fn new() -> Self {
        Self::default()
    }

    fn with_provider(mut self, provider: impl Provider + 'static) -> Self {
        self.providers.push(Box::new(provider));
        self
    }

    fn with_variable(mut self, key: &str, default: Option<&str>) -> Self {
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
            provider_resolver.add_provider(provider);
        }

        Ok(provider_resolver)
    }
}

#[test]
fn if_single_static_provider_with_no_key_to_resolve_is_valid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}

#[test]
fn if_single_static_provider_has_data_for_variable_key_to_resolve_it_succeeds() -> anyhow::Result<()>
{
    let resolver = ResolverTester::new()
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_variable("database_host", None)
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}

#[test]
fn if_there_is_a_single_static_provider_and_it_does_not_contain_a_required_variable_then_validation_fails(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_variable("api_key", None)
        .make_resolver()?;

    assert!(resolver.ensure_required_variables_resolvable().is_err());

    Ok(())
}

#[test]
fn if_there_is_a_dynamic_provider_then_validation_succeeds_even_without_default_value_in_play(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(DynamicProvider)
        .with_variable("api_key", None)
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}

#[test]
fn if_there_is_a_dynamic_provider_and_static_provider_but_the_variable_to_be_resolved_is_not_in_play(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(DynamicProvider)
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_variable("api_key", None)
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}

#[test]
fn if_there_is_a_dynamic_provider_and_a_static_provider_then_validation_succeeds_even_if_there_is_a_variable_in_play(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(DynamicProvider)
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_variable("api_key", Some("super-secret-key"))
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}

#[test]
fn if_there_are_two_static_providers_where_one_has_data_is_valid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_provider(StaticProvider::with_variable(
            "api_key",
            Some("super-secret-key"),
        ))
        .with_variable("database_host", None)
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}
// Ensure that if there are two or more static providers and the first one does not have data for the variable to be resolved,
// but the second or subsequent one does, then validation still succeeds.
#[test]
fn if_there_are_two_static_providers_where_first_provider_does_not_have_data_while_second_provider_does(
) -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_provider(StaticProvider::with_variable(
            "api_key",
            Some("super-secret-key"),
        ))
        .with_variable("api_key", None)
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

    Ok(())
}

#[test]
fn if_there_is_two_static_providers_neither_having_data_is_invalid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_provider(StaticProvider::with_variable(
            "database_host",
            Some("localhost"),
        ))
        .with_provider(StaticProvider::with_variable(
            "api_key",
            Some("super-secret-key"),
        ))
        .with_variable("hello", None)
        .make_resolver()?;

    assert!(resolver.ensure_required_variables_resolvable().is_err());

    Ok(())
}

#[test]
fn no_provider_data_available_but_variable_default_value_needed_is_invalid() -> anyhow::Result<()> {
    let resolver = ResolverTester::new()
        .with_variable("api_key", None)
        .make_resolver()?;

    assert!(resolver.ensure_required_variables_resolvable().is_err());

    Ok(())
}

#[test]
fn no_provider_data_available_but_variable_has_default_value_needed_is_valid() -> anyhow::Result<()>
{
    let resolver = ResolverTester::new()
        .with_variable("api_key", Some("super-secret-key"))
        .make_resolver()?;

    resolver.ensure_required_variables_resolvable()?;

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

#[async_trait]
impl Provider for StaticProvider {
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        Ok(self.variables.get(key.as_str()).cloned().flatten())
    }

    fn may_resolve(&self, key: &Key) -> bool {
        self.variables.contains_key(key.as_str())
    }
}

#[derive(Debug)]
struct DynamicProvider;

#[async_trait]
impl Provider for DynamicProvider {
    async fn get(&self, _key: &Key) -> anyhow::Result<Option<String>> {
        panic!("validation should never call get for a dynamic provider")
    }
}
