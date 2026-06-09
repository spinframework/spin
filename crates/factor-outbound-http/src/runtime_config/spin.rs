use serde::Deserialize;
use spin_factors::runtime_config::toml::GetTomlValue;

/// Get the runtime configuration for outbound HTTP from a TOML table.
///
/// Expects table to be in the format:
/// ```toml
/// [outbound_http]
/// connection_pooling = true # optional, defaults to true
/// max_connections = 10      # optional, defaults to unlimited; 0 = no connections allowed
/// # max_concurrent_requests is deprecated, use max_connections instead
/// ```
pub fn config_from_table(
    table: &impl GetTomlValue,
) -> anyhow::Result<Option<super::RuntimeConfig>> {
    if let Some(outbound_http) = table.get("outbound_http") {
        let toml = outbound_http.clone().try_into::<OutboundHttpToml>()?;

        let max_connections = match (toml.max_connections, toml.max_concurrent_requests) {
            (Some(_), Some(_)) => anyhow::bail!(
                "cannot set both `max_connections` and `max_concurrent_requests` in \
                 `[outbound_http]`; use `max_connections` only"
            ),
            (Some(n), None) => Some(n),
            (None, Some(n)) => {
                terminal::warn!(
                    "`max_concurrent_requests` in `[outbound_http]` is deprecated; \
                     use `max_connections` instead (note: `max_connections = 0` blocks all \
                     connections, whereas `max_concurrent_requests = 0` allowed 1 connection)"
                );
                // Preserve old semaphore semantics: n+1 permits so that 0 allowed 1 connection
                Some(n + 1)
            }
            (None, None) => None,
        };

        Ok(Some(super::RuntimeConfig {
            connection_pooling_enabled: toml.connection_pooling,
            max_concurrent_connections: max_connections,
            wait_timeout: None,
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
    max_connections: Option<usize>,
    /// Deprecated. Use `max_connections` instead.
    #[serde(default)]
    max_concurrent_requests: Option<usize>,
}
