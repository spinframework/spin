use std::any::Any;
use std::marker::PhantomData;

use wasmtime::component::{Linker, ResourceTable};

use crate::{
    prepare::FactorInstanceBuilder, App, AsInstanceState, Error, PrepareContext, RuntimeFactors,
};

/// A contained (i.e., "factored") piece of runtime functionality.
pub trait Factor: Any + Sized {
    /// The particular runtime configuration relevant to this factor.
    ///
    /// Runtime configuration allows for user-provided customization of the
    /// factor's behavior on a per-app basis.
    type RuntimeConfig;

    /// The application state of this factor.
    ///
    /// This state *may* be cached by the runtime across multiple requests.
    type AppState: Sync;

    /// The builder of instance state for this factor.
    type InstanceBuilder: FactorInstanceBuilder;

    /// Initializes this `Factor` for a runtime once at runtime startup.
    ///
    /// This will be called at most once, before any call to
    /// [`Factor::prepare`]. `InitContext` provides access to a wasmtime
    /// `Linker`, so this is where any bindgen `add_to_linker` calls go.
    ///
    /// The type parameter `T` here is the same as the [`wasmtime::Store`] type
    /// parameter `T`, which will contain the [`RuntimeFactors::InstanceState`].
    fn init<C>(&mut self, ctx: &mut C) -> anyhow::Result<()>
    where
        C: InitContext<Self>,
    {
        let _ = ctx;
        Ok(())
    }

    /// Performs factor-specific validation and configuration for the given
    /// [`App`].
    ///
    /// `ConfigureAppContext` gives access to:
    /// - The `spin_app::App`
    /// - This factors's `RuntimeConfig`
    /// - The `AppState` for any factors configured before this one
    ///
    /// A runtime may - but is not required to - reuse the returned config
    /// across multiple instances. Because this method may be called
    /// per-instantiation, it should avoid any blocking operations that could
    /// unnecessarily delay execution.
    ///
    /// This method may be called without any call to `init` or `prepare` in
    /// cases where only validation is needed (e.g., `spin doctor`).
    fn configure_app<T: RuntimeFactors>(
        &self,
        ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState>;

    /// Creates a new `FactorInstanceBuilder`, which will later build
    /// per-instance state for this factor.
    ///
    /// This method is given access to the app component being instantiated and
    /// to any other factors' instance builders that have already been prepared.
    /// As such, this is the primary place for inter-factor dependencies to be
    /// used.
    fn prepare<T: RuntimeFactors>(
        &self,
        ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder>;
}

/// The instance state of the given [`Factor`] `F`.
pub type FactorInstanceState<F> =
    <<F as Factor>::InstanceBuilder as FactorInstanceBuilder>::InstanceState;

/// An InitContext is passed to [`Factor::init`], giving access to the global
/// common [`wasmtime::component::Linker`].
pub trait InitContext<F: Factor> {
    type StoreData: Send + 'static;

    /// Returns a mutable reference to the [`wasmtime::component::Linker`].
    fn linker(&mut self) -> &mut Linker<Self::StoreData>;

    /// Get the instance state for this factor from the store's state.
    fn get_data(store: &mut Self::StoreData) -> &mut FactorInstanceState<F> {
        Self::get_data_with_table(store).0
    }

    /// Get the instance state for this factor from the store's state, with the
    /// resource table as well.
    fn get_data_with_table(
        store: &mut Self::StoreData,
    ) -> (&mut FactorInstanceState<F>, &mut ResourceTable);

    /// Convenience method to link a binding to the linker.
    fn link_bindings(
        &mut self,
        add_to_linker: impl Fn(
            &mut Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut FactorInstanceState<F>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let get_data = Self::get_data;
        add_to_linker(self.linker(), get_data)
    }
}

// used in #[derive(RuntimeFactor)]
#[doc(hidden)]
pub struct FactorInitContext<'a, T, G> {
    pub linker: &'a mut Linker<T>,
    pub _marker: PhantomData<G>,
}

// used in #[derive(RuntimeFactor)]
#[doc(hidden)]
pub trait FactorField {
    type State: crate::RuntimeFactorsInstanceState;
    type Factor: Factor;

    fn get(field: &mut Self::State)
        -> (&mut FactorInstanceState<Self::Factor>, &mut ResourceTable);
}

impl<T, G> InitContext<G::Factor> for FactorInitContext<'_, T, G>
where
    G: FactorField,
    T: AsInstanceState<G::State> + Send + 'static,
{
    type StoreData = T;

    fn linker(&mut self) -> &mut Linker<Self::StoreData> {
        self.linker
    }

    fn get_data_with_table(
        store: &mut Self::StoreData,
    ) -> (&mut FactorInstanceState<G::Factor>, &mut ResourceTable) {
        G::get(store.as_instance_state())
    }
}

pub struct ConfigureAppContext<'a, T: RuntimeFactors, F: Factor> {
    app: &'a App,
    app_state: &'a T::AppState,
    runtime_config: Option<F::RuntimeConfig>,
}

impl<'a, T: RuntimeFactors, F: Factor> ConfigureAppContext<'a, T, F> {
    #[doc(hidden)]
    pub fn new(
        app: &'a App,
        app_state: &'a T::AppState,
        runtime_config: Option<F::RuntimeConfig>,
    ) -> crate::Result<Self> {
        Ok(Self {
            app,
            app_state,
            runtime_config,
        })
    }

    /// Get the [`App`] being configured.
    pub fn app(&self) -> &'a App {
        self.app
    }

    /// Get the app state related to the given factor.
    pub fn app_state<U: Factor>(&self) -> crate::Result<&'a U::AppState> {
        T::app_state::<U>(self.app_state).ok_or(Error::no_such_factor::<U>())
    }

    /// Get a reference to the runtime configuration for the given factor.
    pub fn runtime_config(&self) -> Option<&F::RuntimeConfig> {
        self.runtime_config.as_ref()
    }

    /// Take ownership of the runtime configuration for the given factor.
    pub fn take_runtime_config(&mut self) -> Option<F::RuntimeConfig> {
        self.runtime_config.take()
    }
}

#[doc(hidden)]
pub struct ConfiguredApp<T: RuntimeFactors> {
    app: App,
    app_state: T::AppState,
}

impl<T: RuntimeFactors> ConfiguredApp<T> {
    #[doc(hidden)]
    pub fn new(app: App, app_state: T::AppState) -> Self {
        Self { app, app_state }
    }

    /// Get the configured [`App`].
    pub fn app(&self) -> &App {
        &self.app
    }

    /// Get the configured app's state related to the given factor.
    pub fn app_state<U: Factor>(&self) -> crate::Result<&U::AppState> {
        T::app_state::<U>(&self.app_state).ok_or(Error::no_such_factor::<U>())
    }
}
