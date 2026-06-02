use anyhow::Context as _;
use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound MQTT from a TOML table.
///
/// Expects the table to be in the format:
/// ```toml
/// [outbound_mqtt]
/// max_payload_size_bytes = 65536  # optional, no limit by default
/// max_connections = 10 # optional, defaults to unlimited
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_mqtt) = table.get("outbound_mqtt") {
        let toml = outbound_mqtt
            .clone()
            .try_into::<OutboundMqttToml>()
            .context("failed to parse [outbound_mqtt] table")?;
        Ok(Some(super::RuntimeConfig {
            max_payload_size_bytes: toml.max_payload_size_bytes,
            max_connections: toml.max_connections,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundMqttToml {
    max_payload_size_bytes: Option<usize>,
    max_connections: Option<usize>,
}
