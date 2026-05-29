pub mod spin;

/// Runtime configuration for outbound Redis.
#[derive(Default)]
pub struct RuntimeConfig {
    /// If set, limits the number of concurrent outbound Redis connections.
    pub max_connections: Option<usize>,
}
