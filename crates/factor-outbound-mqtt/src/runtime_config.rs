pub mod spin;

/// Default maximum MQTT payload size: 1 MiB.
///
/// MQTT allows up to 256 MiB
pub const DEFAULT_MAX_PAYLOAD_SIZE_BYTES: usize = 1024 * 1024;

/// Runtime configuration for outbound MQTT.
#[derive(Debug)]
pub struct RuntimeConfig {
    /// Maximum allowed payload size in bytes for outbound MQTT publishes.
    pub max_payload_size_bytes: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_payload_size_bytes: DEFAULT_MAX_PAYLOAD_SIZE_BYTES,
        }
    }
}
