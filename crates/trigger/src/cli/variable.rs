use spin_factor_variables::VariablesFactor;
use spin_factors::RuntimeFactors;
use spin_factors_executor::ExecutorHooks;

/// An executor hook that prepares the variables factor before runtime execution.
pub struct VariablesPreparationExecutorHook;

#[spin_core::async_trait]
impl<F: RuntimeFactors, U> ExecutorHooks<F, U> for VariablesPreparationExecutorHook {
    async fn configure_app(
        &self,
        configured_app: &spin_factors::ConfiguredApp<F>,
    ) -> anyhow::Result<()> {
        let variables_factor = configured_app.app_state::<VariablesFactor>()?;

        let expression_resolver = variables_factor.expression_resolver();
        expression_resolver.pre_runtime_prepare().await?;

        Ok(())
    }
}
