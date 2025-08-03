use spin_core::async_trait;
use spin_factors::RuntimeFactors;
use spin_factors_executor::ExecutorHooks;

/// An [`ExecutorHooks`] that sets variables.
pub struct EnvVariableHook {
    /// The input TOML, for informational summaries.
    pub toml: toml::Table,
}

impl EnvVariableHook {
    pub fn new(toml: toml::Table) -> Self {
        Self {toml}
    }
}

#[async_trait]
impl<F: RuntimeFactors, U> ExecutorHooks<F, U> for  EnvVariableHook {
    async fn configure_app(
        &self,
        _configured_app: &spin_factors::ConfiguredApp<F>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}