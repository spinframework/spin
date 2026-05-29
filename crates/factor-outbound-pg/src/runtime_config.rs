pub mod spin;

/// Runtime configuration for outbound PostgreSQL.
#[derive(Default)]
pub struct RuntimeConfig {
    /// If set, limits the number of concurrent outbound PostgreSQL connections.
    pub max_connections: Option<usize>,
}
