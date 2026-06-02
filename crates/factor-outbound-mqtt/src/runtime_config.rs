pub mod spin;

/// Runtime configuration for outbound MQTT.
#[derive(Debug, Default)]
pub struct RuntimeConfig {
    /// Maximum allowed payload size in bytes for outbound MQTT publishes.
    ///
    /// When `None` (the default), no limit is enforced. Operators in multi-tenant deployments
    /// should set this to prevent tenants from sending excessively large payloads.
    /// Configure via `[outbound_mqtt] max_payload_size_bytes` in the runtime config TOML.
    pub max_payload_size_bytes: Option<usize>,
}
