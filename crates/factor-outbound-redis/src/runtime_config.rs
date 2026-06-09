pub mod spin;

/// Runtime configuration for outbound Redis.
#[derive(Default)]
pub struct RuntimeConfig {
    /// If set, limits the number of concurrent outbound Redis connections.
    pub max_connections: Option<usize>,
    /// If set, limits how long `acquire` will wait for a connection permit.
    pub wait_timeout: Option<std::time::Duration>,
}
