use std::sync::Arc;

use spin_factor_outbound_networking::config::allowed_hosts::OutboundAllowedHosts;
use spin_world::spin::postgres4_2_0::postgres::{self as v4};

/// Encapsulates checking of a PostgreSQL address/connection string against
/// an allow-list.
///
/// This is broken out as a distinct object to allow it to be synchronously retrieved
/// within a P3 Accessor block and then asynchronously queried outside the block.
#[derive(Clone)]
pub(crate) struct AllowedHostChecker {
    allowed_hosts: Arc<OutboundAllowedHosts>,
}

impl AllowedHostChecker {
    pub fn new(allowed_hosts: OutboundAllowedHosts) -> Self {
        Self {
            allowed_hosts: Arc::new(allowed_hosts),
        }
    }
    #[allow(clippy::result_large_err)]
    pub async fn ensure_address_allowed(&self, address: &str) -> Result<(), v4::Error> {
        fn conn_failed(message: impl Into<String>) -> v4::Error {
            v4::Error::ConnectionFailed(message.into())
        }
        fn err_other(err: anyhow::Error) -> v4::Error {
            v4::Error::Other(err.to_string())
        }

        let config = address
            .parse::<tokio_postgres::Config>()
            .map_err(|e| conn_failed(e.to_string()))?;

        for (i, host) in config.get_hosts().iter().enumerate() {
            match host {
                tokio_postgres::config::Host::Tcp(address) => {
                    let ports = config.get_ports();
                    // The port we use is either:
                    // * The port at the same index as the host
                    // * The first port if there is only one port
                    let port = ports.get(i).or_else(|| {
                        if ports.len() == 1 {
                            ports.first()
                        } else {
                            None
                        }
                    });
                    let port_str = port.map(|p| format!(":{p}")).unwrap_or_default();
                    let url = format!("{address}{port_str}");
                    if !self
                        .allowed_hosts
                        .check_url(&url, "postgres")
                        .await
                        .map_err(err_other)?
                    {
                        return Err(conn_failed(format!(
                            "address postgres://{url} is not permitted"
                        )));
                    }
                }
                #[cfg(unix)]
                tokio_postgres::config::Host::Unix(_) => {
                    return Err(conn_failed("Unix sockets are not supported on WebAssembly"));
                }
            }
        }
        Ok(())
    }
}
