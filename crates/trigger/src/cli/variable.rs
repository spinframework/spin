use spin_factor_variables::VariablesFactor;
use spin_factors::RuntimeFactors;
use spin_factors_executor::ExecutorHooks;

/// An executor hook that prepares the variables factor before runtime execution.
pub struct VariablesValidatorHook;

#[spin_core::async_trait]
impl<F: RuntimeFactors, U> ExecutorHooks<F, U> for VariablesValidatorHook {
    async fn configure_app(
        &self,
        configured_app: &spin_factors::ConfiguredApp<F>,
    ) -> anyhow::Result<()> {
        let variables_factor_app_state = configured_app.app_state::<VariablesFactor>()?;

        let expression_resolver = variables_factor_app_state.expression_resolver();
        expression_resolver.validate_variables().await?;

        Ok(())
    }
}
