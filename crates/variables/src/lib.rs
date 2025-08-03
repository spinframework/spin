//! The runtime configuration for the variables factor used in the Spin CLI.

mod azure_key_vault;
mod env;
mod statik;
mod vault;

pub use azure_key_vault::*;
pub use env::*;
pub use statik::*;
pub use vault::*;

use serde::Deserialize;
use spin_expressions::Provider;
use spin_factors::{anyhow, runtime_config::toml::GetTomlValue};

use spin_factor_variables::runtime_config::RuntimeConfig;

/// Resolves a runtime configuration for the variables factor from a TOML table.
pub fn runtime_config_from_toml(table: &impl GetTomlValue) -> anyhow::Result<RuntimeConfig> {
    // Always include the environment variable provider.
    let var_provider = vec![Box::<EnvVariablesProvider>::default() as _];
    let value = table
        .get("variables_provider")
        .or_else(|| table.get("config_provider"));
    let Some(array) = value else {
        return Ok(RuntimeConfig {
            providers: var_provider,
        });
    };

    let provider_configs: Vec<VariableProviderConfiguration> = array.clone().try_into()?;
    let mut providers = provider_configs
        .into_iter()
        .map(VariableProviderConfiguration::into_provider)
        .collect::<anyhow::Result<Vec<_>>>()?;
    providers.extend(var_provider);
    Ok(RuntimeConfig { providers })
}

/// A runtime configuration used in the Spin CLI for one type of variable provider.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum VariableProviderConfiguration {
    /// A provider that uses Azure Key Vault.
    AzureKeyVault(AzureKeyVaultVariablesConfig),
    /// A static provider of variables.
    Static(StaticVariablesProvider),
    /// A provider that uses HashiCorp Vault.
    Vault(VaultVariablesProvider),
    /// An environment variable provider.
    Env(EnvVariablesConfig),
}

impl VariableProviderConfiguration {
    /// Returns the provider for the configuration.
    pub fn into_provider(self) -> anyhow::Result<Box<dyn Provider>> {
        let provider: Box<dyn Provider> = match self {
            VariableProviderConfiguration::Static(provider) => Box::new(provider),
            VariableProviderConfiguration::Env(config) => Box::new(env::EnvVariablesProvider::new(
                config.prefix,
                |s| std::env::var(s),
                config.dotenv_path,
            )),
            VariableProviderConfiguration::Vault(provider) => Box::new(provider),
            VariableProviderConfiguration::AzureKeyVault(config) => Box::new(
                AzureKeyVaultProvider::create(config.vault_url.clone(), config.try_into()?)?,
            ),
        };
        Ok(provider)
    }
}
