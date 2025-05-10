mod factor;
mod prepare;
pub mod runtime_config;
mod runtime_factors;

pub use anyhow;
pub use serde;
pub use wasmtime;

pub use spin_app::{App, AppComponent};
pub use spin_factors_derive::RuntimeFactors;

pub use crate::{
    factor::{
        ConfigureAppContext, ConfiguredApp, Factor, FactorField, FactorInitContext,
        FactorInstanceState, InitContext,
    },
    prepare::{FactorInstanceBuilder, PrepareContext, SelfInstanceBuilder},
    runtime_config::{FactorRuntimeConfigSource, RuntimeConfigSourceFinalizer},
    runtime_factors::{
        AsInstanceState, HasInstanceBuilder, RuntimeFactors, RuntimeFactorsInstanceState,
    },
};

/// Result wrapper type defaulting to use [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("two or more factors share the same type: {0}")]
    DuplicateFactorTypes(String),
    #[error("factor dependency ordering error: {0}")]
    DependencyOrderingError(String),
    #[error("{factor}::InstanceBuilder::build failed: {source}")]
    FactorBuildError {
        factor: &'static str,
        source: anyhow::Error,
    },
    #[error("{factor}::configure_app failed: {source}")]
    FactorConfigureAppError {
        factor: &'static str,
        source: anyhow::Error,
    },
    #[error("{factor}::init failed: {source}")]
    FactorInitError {
        factor: &'static str,
        source: anyhow::Error,
    },
    #[error("{factor}::prepare failed: {source}")]
    FactorPrepareError {
        factor: &'static str,
        source: anyhow::Error,
    },
    #[error("no such factor: {0}")]
    NoSuchFactor(&'static str),
    #[error("{factor} requested already-consumed key {key:?}")]
    RuntimeConfigReusedKey { factor: &'static str, key: String },
    #[error("runtime config error: {0}")]
    RuntimeConfigSource(#[source] anyhow::Error),
    #[error("unused runtime config key(s): {}", keys.join(", "))]
    RuntimeConfigUnusedKeys { keys: Vec<String> },
    #[error("unknown component: {0}")]
    UnknownComponent(String),
}

impl Error {
    fn no_such_factor<T: Factor>() -> Self {
        Self::NoSuchFactor(std::any::type_name::<T>())
    }

    pub fn runtime_config_reused_key<T: Factor>(key: impl Into<String>) -> Self {
        Self::RuntimeConfigReusedKey {
            factor: std::any::type_name::<T>(),
            key: key.into(),
        }
    }

    // These helpers are used by factors-derive

    #[doc(hidden)]
    pub fn factor_init_error<T: Factor>(source: anyhow::Error) -> Self {
        let factor = std::any::type_name::<T>();
        Self::FactorInitError { factor, source }
    }

    #[doc(hidden)]
    pub fn factor_configure_app_error<T: Factor>(source: anyhow::Error) -> Self {
        let factor = std::any::type_name::<T>();
        Self::FactorConfigureAppError { factor, source }
    }

    #[doc(hidden)]
    pub fn factor_prepare_error<T: Factor>(source: anyhow::Error) -> Self {
        let factor = std::any::type_name::<T>();
        Self::FactorPrepareError { factor, source }
    }

    #[doc(hidden)]
    pub fn factor_build_error<T: Factor>(source: anyhow::Error) -> Self {
        let factor = std::any::type_name::<T>();
        Self::FactorBuildError { factor, source }
    }
}
