use spin_core::async_trait;
use spin_factors::RuntimeFactors;
use spin_factors_executor::ExecutorHooks;
use spin_variables::{VariableProviderConfiguration, VariableSourcer};

/// Implements TriggerHooks, sorting required variables
pub struct VariableSorterExecutorHooks {
    table: toml::Table,
}

impl VariableSorterExecutorHooks {
    pub fn new(table: toml::Table) -> Self {
        Self { table }
    }
}

#[async_trait]
impl<F: RuntimeFactors, U> ExecutorHooks<F, U> for VariableSorterExecutorHooks {
    async fn configure_app(
        &self,
        configured_app: &spin_factors::ConfiguredApp<F>,
    ) -> anyhow::Result<()> {
        for (key, variable) in configured_app.app().variables() {
            self.variable_env_checker(key.clone(), variable.clone())?;
        }
        Ok(())
    }
}

impl VariableSourcer for VariableSorterExecutorHooks {
    fn variable_env_checker(&self, key: String, val: spin_app::Variable) -> anyhow::Result<()> {
        let configs = spin_variables::variable_provider_config_from_toml(&self.table)?;

        if let Some(config) = configs.into_iter().next() {
            let (dotenv_path, prefix) = match config {
                VariableProviderConfiguration::Env(env) => (env.dotenv_path, env.prefix),
                _ => (None, None),
            };
            return self.check(key, val, dotenv_path, prefix);
        }

        Err(anyhow::anyhow!("No environment variable provider found"))
    }
}
