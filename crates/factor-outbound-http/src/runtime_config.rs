#[cfg(feature = "spin-cli")]
pub mod spin;

/// Runtime configuration for outbound HTTP.
#[derive(Debug)]
pub struct RuntimeConfig {
    /// If true, enable connection pooling and reuse.
    pub connection_pooling_enabled: bool,
    /// If set, limits the number of concurrent outbound connections.
    pub max_concurrent_connections: Option<usize>,
    /// If set, limits how long `acquire` will wait for a connection permit.
    pub wait_timeout: Option<std::time::Duration>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            connection_pooling_enabled: true,
            max_concurrent_connections: None,
            wait_timeout: None,
        }
    }
}
