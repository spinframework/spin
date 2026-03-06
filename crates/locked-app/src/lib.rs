//! Spin internal application interfaces
//!
//! This crate contains interfaces to Spin application configuration to be used
//! by crates that implement Spin execution environments: trigger executors and
//! host components, in particular.

#![deny(missing_docs)]

pub mod locked;
mod metadata;
pub mod values;

pub use async_trait::async_trait;
pub use locked::Variable;
pub use metadata::{MetadataExt, MetadataKey};

/// MetadataKey for extracting the application name.
pub const APP_NAME_KEY: MetadataKey = MetadataKey::new("name");
/// MetadataKey for extracting the application version.
pub const APP_VERSION_KEY: MetadataKey = MetadataKey::new("version");
/// MetadataKey for extracting the application description.
pub const APP_DESCRIPTION_KEY: MetadataKey = MetadataKey::new("description");
/// MetadataKey for extracting the OCI image digest.
pub const OCI_IMAGE_DIGEST_KEY: MetadataKey = MetadataKey::new("oci_image_digest");
/// MetadataKey for extracting whether a component is stateful.
pub const STATEFUL_KEY: MetadataKey<bool> = MetadataKey::new("stateful");

/// Type alias for a [`Result`]s with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by methods in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An error propagated from the `spin_core` crate.
    #[error(transparent)]
    CoreError(anyhow::Error),
    /// An error from a `DynamicHostComponent`.
    #[error("host component error: {0:#}")]
    HostComponentError(#[source] anyhow::Error),
    /// An error from a `Loader` implementation.
    #[error(transparent)]
    LoaderError(anyhow::Error),
    /// An error indicating missing or unexpected metadata.
    #[error("metadata error: {0}")]
    MetadataError(String),
    /// An error indicating failed JSON (de)serialization.
    #[error("json error: {0}")]
    JsonError(#[from] serde_json::Error),
    /// A validation error that can be presented directly to the user.
    #[error(transparent)]
    ValidationError(anyhow::Error),
}
