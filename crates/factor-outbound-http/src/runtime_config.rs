#[cfg(feature = "spin-cli")]
pub mod spin;

/// Runtime configuration for outbound HTTP.
#[derive(Debug)]
pub struct RuntimeConfig {
    /// If true, enable connection pooling and reuse.
    pub connection_pooling: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            connection_pooling: true,
        }
    }
}
