/// Type alias for a [`Result`]s with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by methods in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An error propagated from the `spin_core` crate.
    #[error(transparent)]
    Core(anyhow::Error),
    /// An error from a `DynamicHostComponent`.
    #[error("host component error: {0:#}")]
    HostComponent(#[source] anyhow::Error),
    /// An error from a `Loader` implementation.
    #[error(transparent)]
    Loader(anyhow::Error),
    /// An error indicating missing or unexpected metadata.
    #[error("metadata error: {0}")]
    Metadata(String),
    /// An error indicating failed JSON (de)serialization.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// A validation error that can be presented directly to the user.
    #[error(transparent)]
    Validation(anyhow::Error),
}
