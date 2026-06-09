use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound MySQL from a TOML table.
///
/// Expects table to be in the format:
/// ```toml
/// [outbound_mysql]
/// max_connections = 10 # optional, defaults to unlimited
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_mysql) = table.get("outbound_mysql") {
        let toml = outbound_mysql.clone().try_into::<OutboundMysqlToml>()?;
        Ok(Some(super::RuntimeConfig {
            max_connections: toml.max_connections,
            wait_timeout: None,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundMysqlToml {
    #[serde(default)]
    max_connections: Option<usize>,
}
