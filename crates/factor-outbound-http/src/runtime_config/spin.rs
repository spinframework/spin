use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound HTTP from a TOML table.
///
/// Expects table to be in the format:
/// ```toml
/// [outbound_http]
/// connection_pooling = true
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_http) = table.get("outbound_http") {
        Ok(Some(super::RuntimeConfig {
            connection_pooling: outbound_http
                .clone()
                .try_into::<OutboundHttpToml>()?
                .connection_pooling,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundHttpToml {
    #[serde(default)]
    connection_pooling: bool,
}
