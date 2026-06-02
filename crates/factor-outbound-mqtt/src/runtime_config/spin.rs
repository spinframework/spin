use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound MQTT from a TOML table.
///
/// Expects table to be in the format:
/// ```toml
/// [outbound_mqtt]
/// max_connections = 10 # optional, defaults to unlimited
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_mqtt) = table.get("outbound_mqtt") {
        let toml = outbound_mqtt.clone().try_into::<OutboundMqttToml>()?;
        Ok(Some(super::RuntimeConfig {
            max_connections: toml.max_connections,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundMqttToml {
    #[serde(default)]
    max_connections: Option<usize>,
}
