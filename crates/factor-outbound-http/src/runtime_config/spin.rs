use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound HTTP from a TOML table.
///
/// Expects table to be in the format:
/// ```toml
/// [outbound_http]
/// connection_pooling = true # optional, defaults to true
/// max_concurrent_requests = 10 # optional, defaults to unlimited
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_http) = table.get("outbound_http") {
        let outbound_http_toml = outbound_http.clone().try_into::<OutboundHttpToml>()?;
        Ok(Some(super::RuntimeConfig {
            connection_pooling_enabled: outbound_http_toml.connection_pooling,
            max_concurrent_requests: outbound_http_toml.max_concurrent_requests,
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
    #[serde(default)]
    max_concurrent_requests: Option<usize>,
}
