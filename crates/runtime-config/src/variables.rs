//! The runtime configuration for the variables factor used in the Spin CLI.

mod azure_key_vault;
mod env;
mod statik;
mod vault;

use std::path::PathBuf;

pub use azure_key_vault::*;
pub use env::*;
use spin_common::{env::env_key, ui::quoted_path};
use spin_locked_app::Variable;
pub use statik::*;
pub use vault::*;

use serde::Deserialize;
use spin_expressions::Provider;
use spin_factor_variables::runtime_config::RuntimeConfig;
use spin_factors::runtime_config::toml::GetTomlValue;
use spin_variables_azure::{AzureKeyVaultProvider, AzureKeyVaultVariablesConfig};
use spin_variables_env::{EnvVariablesConfig, EnvVariablesProvider};
use spin_variables_static::StaticVariablesProvider;
use spin_variables_vault::VaultVariablesProvider;

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

pub fn variable_provider_config_from_toml(
    table: &impl GetTomlValue,
) -> anyhow::Result<Vec<VariableProviderConfiguration>> {
    if let Some(array) = table
        .get("variables_provider")
        .or_else(|| table.get("config_provider"))
    {
        array
            .clone()
            .try_into::<Vec<VariableProviderConfiguration>>()
            .map_err(|e| anyhow::anyhow!("Failed to parse variable provider configuration: {}", e))
    } else {
        Ok(vec![VariableProviderConfiguration::Env(
            EnvVariablesConfig::default(),
        )])
    }
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
            VariableProviderConfiguration::Env(config) => Box::new(EnvVariablesProvider::new(
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

pub trait VariableSourcer {
    fn variable_env_checker(&self, key: String, val: Variable) -> anyhow::Result<()>;

    fn check(
        &self,
        key: String,
        mut val: Variable,
        dotenv_path: Option<PathBuf>,
        prefix: Option<String>,
    ) -> anyhow::Result<()> {
        if val.default.is_some() {
            return Ok(());
        }

        if let Some(path) = dotenv_path {
            _ = std::env::set_current_dir(path);
        }

        match std::env::var(env_key(prefix, &key)) {
            Ok(v) => {
                val.default = Some(v);
                Ok(())
            }
            Err(_) => Err(anyhow::anyhow!(
                "Variable data not provided for {}",
                quoted_path(key)
            )),
        }
    }
}
