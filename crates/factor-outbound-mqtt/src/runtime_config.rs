pub mod spin;

/// Runtime configuration for outbound MQTT.
#[derive(Default)]
pub struct RuntimeConfig {
    /// If set, limits the number of concurrent outbound MQTT connections.
    pub max_connections: Option<usize>,
}
