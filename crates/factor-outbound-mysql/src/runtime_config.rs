pub mod spin;

/// Runtime configuration for outbound MySQL.
#[derive(Default)]
pub struct RuntimeConfig {
    /// If set, limits the number of concurrent outbound MySQL connections.
    pub max_connections: Option<usize>,
}
