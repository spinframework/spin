#[cfg(feature = "spin-cli")]
pub mod spin;

/// Runtime configuration for outbound HTTP.
#[derive(Debug)]
pub struct RuntimeConfig {
    /// If true, enable connection pooling and reuse.
    pub connection_pooling_enabled: bool,
    /// If set, limits the number of concurrent outbound requests.
    pub max_concurrent_requests: Option<usize>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            connection_pooling_enabled: true,
            max_concurrent_requests: None,
        }
    }
}
