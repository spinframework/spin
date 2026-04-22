use wasmtime::component::{Linker, ResourceTable};

use crate::{App, ConfiguredApp, Factor, factor::FactorInstanceState};

/// A collection of `Factor`s that are initialized and configured together.
///
/// Implemented by `#[derive(RuntimeFactors)]` and should not be implemented manually.
///
/// # Example
///
/// A typical usage of `RuntimeFactors` would look something like the following pseudo-code:
///
/// ```ignore
/// #[derive(RuntimeFactors)]
/// struct MyFactors {
///  // ...
/// }
/// // Initialize the factors collection
/// let factors = MyFactors { /* .. */ };
/// // Initialize each factor with a linker
/// factors.init(&mut linker)?;
/// // Configure the factors with an app and runtime config
/// let configured_app = factors.configure_app(app, runtime_config)?;
/// // Prepare instance state builders
/// let builders = factors.prepare(&configured_app, "component-id")?;
/// // Build the instance state for the factors
/// let data = factors.build_instance_state(builders)?;
/// // Initialize a `wasmtime` store with the instance state
/// let mut store = wasmtime::Store::new(&engine, data);
/// // Instantiate the component
/// let instance = linker.instantiate_async(&mut store, &component).await?;
/// ```
pub trait RuntimeFactors: Send + Sync + Sized + 'static {
    /// The per application state of all the factors.
    type AppState: Sync + Send;
    /// The per instance state of the factors.
    type InstanceState: RuntimeFactorsInstanceState;
    /// The collection of all the `InstanceBuilder`s of the factors.
    type InstanceBuilders: Send + HasInstanceBuilder;
    /// The runtime configuration of all the factors.
    type RuntimeConfig: Default;

    /// Initialize the factors with the given linker.
    ///
    /// Each factor's `init` is called in turn. Must be called once before
    /// [`RuntimeFactors::prepare`].
    fn init<T: AsInstanceState<Self::InstanceState> + Send + 'static>(
        &mut self,
        linker: &mut Linker<T>,
    ) -> crate::Result<()>;

    /// Configure the factors with the given app and runtime config.
    fn configure_app(
        &self,
        app: App,
        runtime_config: Self::RuntimeConfig,
    ) -> crate::Result<ConfiguredApp<Self>>;

    /// Prepare the factors' instance state builders.
    fn prepare(
        &self,
        configured_app: &ConfiguredApp<Self>,
        component_id: &str,
    ) -> crate::Result<Self::InstanceBuilders>;

    /// Build the instance state for the factors.
    fn build_instance_state(
        &self,
        builders: Self::InstanceBuilders,
    ) -> crate::Result<Self::InstanceState>;

    /// Get the app state related to a particular factor.
    fn app_state<F: Factor>(app_state: &Self::AppState) -> Option<&F::AppState>;

    /// Get the instance builder of a particular factor.
    ///
    /// The outer `Option` is `None` if the factor has not been registered with this `Factors` collection,
    /// and the inner `Option` is `None` if the factor has not been prepared yet.
    fn instance_builder_mut<F: Factor>(
        builders: &mut Self::InstanceBuilders,
    ) -> Option<Option<&mut F::InstanceBuilder>>;
}

/// Allows querying an `InstanceBuilders` for a particular `Factor`'s `InstanceBuilder`.
pub trait HasInstanceBuilder {
    /// Get the instance builder of a particular factor.
    fn for_factor<F: Factor>(&mut self) -> Option<&mut F::InstanceBuilder>;
}

/// Get the state of a particular Factor from the overall InstanceState
///
/// Implemented by `#[derive(RuntimeFactors)]`
pub trait RuntimeFactorsInstanceState: AsInstanceState<Self> + Send + 'static {
    fn get_with_table<F: Factor>(
        &mut self,
    ) -> Option<(&mut FactorInstanceState<F>, &mut ResourceTable)>;

    fn get<F: Factor>(&mut self) -> Option<&mut FactorInstanceState<F>> {
        self.get_with_table::<F>().map(|(state, _)| state)
    }

    fn table(&self) -> &ResourceTable;

    fn table_mut(&mut self) -> &mut ResourceTable;
}

pub trait AsInstanceState<T: RuntimeFactorsInstanceState + ?Sized> {
    fn as_instance_state(&mut self) -> &mut T;
}
