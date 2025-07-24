use std::any::Any;

use spin_app::AppComponent;

use crate::{Error, Factor, RuntimeFactors};

/// A builder for a [`Factor`]'s per instance state.
pub trait FactorInstanceBuilder: Any {
    /// The per instance state of the factor.
    ///
    /// This is equivalent to the existing `HostComponent::Data` and ends up
    /// being stored in the `wasmtime::Store`. Any `bindgen` traits for this
    /// factor will be implemented on this type.
    type InstanceState: Send + 'static;

    /// Build the per instance state of the factor.
    fn build(self) -> anyhow::Result<Self::InstanceState>;
}

impl FactorInstanceBuilder for () {
    type InstanceState = ();

    fn build(self) -> anyhow::Result<Self::InstanceState> {
        Ok(())
    }
}

/// A helper trait for when the type implementing [`FactorInstanceBuilder`] is also the instance state.
pub trait SelfInstanceBuilder: Send + 'static {}

impl<T: SelfInstanceBuilder> FactorInstanceBuilder for T {
    type InstanceState = Self;

    fn build(self) -> anyhow::Result<Self::InstanceState> {
        Ok(self)
    }
}

/// A PrepareContext is passed to [`Factor::prepare`].
///
/// This gives the factor access to app state and the app component.
pub struct PrepareContext<'a, T: RuntimeFactors, F: Factor> {
    pub(crate) app_state: &'a F::AppState,
    pub(crate) app_component: &'a AppComponent<'a>,
    pub(crate) instance_builders: &'a mut T::InstanceBuilders,
}

impl<'a, T: RuntimeFactors, F: Factor> PrepareContext<'a, T, F> {
    #[doc(hidden)]
    pub fn new(
        app_state: &'a F::AppState,
        app_component: &'a AppComponent,
        instance_builders: &'a mut T::InstanceBuilders,
    ) -> Self {
        Self {
            app_state,
            app_component,
            instance_builders,
        }
    }

    /// Get the app state related to the factor.
    pub fn app_state(&self) -> &'a F::AppState {
        self.app_state
    }

    /// Get the app component.
    pub fn app_component(&self) -> &'a AppComponent<'_> {
        self.app_component
    }

    /// Returns the prepared [`FactorInstanceBuilder`] for the given [`Factor`].
    ///
    /// Fails if the current [`RuntimeFactors`] does not include the given
    /// [`Factor`] or if the given [`Factor`]'s builder has not been prepared
    /// yet (because it is sequenced after this factor).
    pub fn instance_builder<U: Factor>(&mut self) -> crate::Result<&mut U::InstanceBuilder> {
        T::instance_builder_mut::<U>(self.instance_builders)
            .ok_or(Error::no_such_factor::<U>())?
            .ok_or_else(|| {
                Error::DependencyOrderingError(format!(
                    "{factor} builder requested before it was prepared",
                    factor = std::any::type_name::<U>()
                ))
            })
    }
}
