use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound PostgreSQL from a TOML table.
///
/// Expects table to be in the format:
/// ```toml
/// [outbound_pg]
/// max_connections = 10 # optional, defaults to unlimited
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_pg) = table.get("outbound_pg") {
        let toml = outbound_pg.clone().try_into::<OutboundPgToml>()?;
        Ok(Some(super::RuntimeConfig {
            max_connections: toml.max_connections,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutboundPgToml {
    #[serde(default)]
    max_connections: Option<usize>,
}
