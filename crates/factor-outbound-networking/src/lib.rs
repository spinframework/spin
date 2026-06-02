mod allowed_hosts;
pub mod connection_semaphore;
pub mod runtime_config;
mod tls;

use std::{collections::HashMap, sync::Arc};

use futures_util::FutureExt as _;
use opentelemetry_semantic_conventions::attribute::SERVER_PORT;
use spin_factor_variables::VariablesFactor;
use spin_factor_wasi::{SocketAddrUse, SocketPermitState, WasiFactor};
use spin_factors::{
    ConfigureAppContext, Error, Factor, FactorInstanceBuilder, PrepareContext, RuntimeFactors,
    anyhow::{self, Context},
};
use spin_outbound_networking_config::allowed_hosts::{DisallowedHostHandler, OutboundAllowedHosts};
use tokio::sync::Semaphore;
use url::Url;

use crate::{
    allowed_hosts::allowed_outbound_hosts, runtime_config::RuntimeConfig, tls::TlsClientConfigs,
};
pub use allowed_hosts::validate_service_chaining_for_components;
pub use connection_semaphore::{ConnectionPermit, ConnectionSemaphore};

pub use crate::tls::{ComponentTlsClientConfigs, TlsClientConfig};
use config::allowed_hosts::AllowedHostsConfig;
use config::blocked_networks::BlockedNetworks;
pub use spin_outbound_networking_config as config;

#[derive(Default)]
pub struct OutboundNetworkingFactor {
    disallowed_host_handler: Option<Arc<dyn DisallowedHostHandler>>,
}

impl OutboundNetworkingFactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a handler to be called when a request is disallowed by an
    /// instance's configured `allowed_outbound_hosts`.
    pub fn set_disallowed_host_handler(&mut self, handler: impl DisallowedHostHandler + 'static) {
        self.disallowed_host_handler = Some(Arc::new(handler));
    }
}

impl Factor for OutboundNetworkingFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceBuilder;

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        // Extract allowed_outbound_hosts for all components
        let component_allowed_hosts = ctx
            .app()
            .components()
            .map(|component| {
                Ok((
                    component.id().to_string(),
                    allowed_outbound_hosts(&component)?
                        .into_boxed_slice()
                        .into(),
                ))
            })
            .collect::<anyhow::Result<_>>()?;

        let RuntimeConfig {
            client_tls_configs,
            blocked_ip_networks: block_networks,
            block_private_networks,
            max_socket_connections,
            max_total_connections,
        } = ctx.take_runtime_config().unwrap_or_default();

        let blocked_networks = BlockedNetworks::new(block_networks, block_private_networks);
        let tls_client_configs = TlsClientConfigs::new(client_tls_configs)?;
        let global_connection_semaphore =
            max_total_connections.map(|n| Arc::new(Semaphore::new(n)));

        if let (Some(socket_cap), Some(global_cap)) =
            (max_socket_connections, max_total_connections)
            && socket_cap > global_cap
        {
            tracing::warn!(
                "outbound_networking max_socket_connections ({socket_cap}) exceeds \
                 max_total_connections ({global_cap}); the global limit will be the effective \
                 cap for TCP/UDP sockets"
            );
        }

        let socket_connection_semaphore =
            if max_socket_connections.is_some() || global_connection_semaphore.is_some() {
                Some(ConnectionSemaphore::new(
                    global_connection_semaphore.clone(),
                    max_socket_connections,
                    "wasi-sockets",
                ))
            } else {
                None
            };

        Ok(AppState {
            component_allowed_hosts,
            blocked_networks,
            tls_client_configs,
            socket_connection_semaphore,
            global_connection_semaphore,
            max_total_connections,
        })
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        mut ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let hosts = ctx
            .app_state()
            .component_allowed_hosts
            .get(ctx.app_component().id())
            .cloned()
            .context("missing component allowed hosts")?;
        let resolver = ctx
            .instance_builder::<VariablesFactor>()?
            .expression_resolver()
            .clone();
        let component_ids = ctx
            .app_component()
            .app
            .components()
            .map(|c| c.id().to_string())
            .collect::<Vec<_>>();
        let allowed_hosts_future = async move {
            let prepared = resolver.prepare().await.inspect_err(|err| {
                tracing::error!(
                    %err, "error.type" = "variable_resolution_failed",
                    "Error resolving variables when checking request against allowed outbound hosts",
                );
            })?;
            AllowedHostsConfig::parse(&hosts, &prepared, &component_ids).inspect_err(|err| {
                tracing::error!(
                    %err, "error.type" = "invalid_allowed_hosts",
                    "Error parsing allowed outbound hosts",
                );
            })
        }
        .map(|res| res.map(Arc::new).map_err(Arc::new))
        .boxed()
        .shared();
        let allowed_hosts = OutboundAllowedHosts::new(
            allowed_hosts_future.clone(),
            self.disallowed_host_handler.clone(),
        );
        let blocked_networks = ctx.app_state().blocked_networks.clone();
        let permit_state = ctx
            .app_state()
            .socket_connection_semaphore
            .clone()
            .map(SocketPermitState::new);

        match ctx.instance_builder::<WasiFactor>() {
            Ok(wasi_builder) => {
                if let Some(state) = permit_state {
                    wasi_builder.set_socket_permit_state(state);
                }

                let allowed_hosts = allowed_hosts.clone();
                wasi_builder.outbound_socket_addr_check(move |addr, addr_use| {
                    let allowed_hosts = allowed_hosts.clone();
                    let blocked_networks = blocked_networks.clone();
                    async move {
                        let scheme = match addr_use {
                            SocketAddrUse::TcpBind => return false,
                            SocketAddrUse::TcpConnect => "tcp",
                            SocketAddrUse::UdpBind
                            | SocketAddrUse::UdpConnect
                            | SocketAddrUse::UdpOutgoingDatagram => "udp",
                        };
                        if !allowed_hosts
                            .check_url(&addr.to_string(), scheme)
                            .await
                            .unwrap_or(
                                // TODO: should this trap (somehow)?
                                false,
                            )
                        {
                            return false;
                        }
                        if blocked_networks.is_blocked(&addr) {
                            tracing::error!(
                                "error.type" = "destination_ip_prohibited",
                                ?addr,
                                "destination IP prohibited by runtime config"
                            );
                            return false;
                        }
                        true
                    }
                });
            }
            Err(Error::NoSuchFactor(_)) => (), // no WasiFactor to configure; that's OK
            Err(err) => return Err(err.into()),
        }

        let component_tls_configs = ctx
            .app_state()
            .tls_client_configs
            .get_component_tls_configs(ctx.app_component().id());

        Ok(InstanceBuilder {
            allowed_hosts,
            blocked_networks: ctx.app_state().blocked_networks.clone(),
            component_tls_client_configs: component_tls_configs,
        })
    }
}

pub struct AppState {
    /// Component ID -> Allowed host list
    component_allowed_hosts: HashMap<String, Arc<[String]>>,
    /// Blocked IP networks
    blocked_networks: BlockedNetworks,
    /// TLS client configs
    tls_client_configs: TlsClientConfigs,
    /// Pre-built semaphore for TCP/UDP socket quota enforcement (global + socket-specific).
    /// `None` means no limits are configured.
    socket_connection_semaphore: Option<ConnectionSemaphore>,
    /// App-wide semaphore capping total concurrent outbound connections across ALL types.
    /// `None` means unlimited.
    global_connection_semaphore: Option<Arc<Semaphore>>,
    /// The configured global connection limit (for warning comparisons in other factors).
    max_total_connections: Option<usize>,
}

/// Builds a [`ConnectionSemaphore`] for an outbound factor, incorporating the optional global
/// connection limit from the networking factor's app state.
///
/// Emits a warning when the per-factor limit exceeds the global cap (the global limit would
/// be the effective ceiling in that case).
pub fn build_connection_semaphore(
    networking: Option<&AppState>,
    factor: &'static str,
    factor_limit: Option<usize>,
) -> ConnectionSemaphore {
    if let (Some(per_factor), Some(global_limit)) = (
        factor_limit,
        networking.and_then(|n| n.max_total_connections),
    ) && per_factor > global_limit
    {
        tracing::warn!(
            "outbound_{factor} max_connections ({per_factor}) exceeds global \
             max_total_connections ({global_limit}); the global limit will be the \
             effective cap"
        );
    }
    ConnectionSemaphore::new(
        networking.and_then(|n| n.global_connection_semaphore.clone()),
        factor_limit,
        factor,
    )
}

pub struct InstanceBuilder {
    allowed_hosts: OutboundAllowedHosts,
    blocked_networks: BlockedNetworks,
    component_tls_client_configs: ComponentTlsClientConfigs,
}

impl InstanceBuilder {
    pub fn allowed_hosts(&self) -> OutboundAllowedHosts {
        self.allowed_hosts.clone()
    }

    pub fn blocked_networks(&self) -> BlockedNetworks {
        self.blocked_networks.clone()
    }

    pub fn component_tls_configs(&self) -> ComponentTlsClientConfigs {
        self.component_tls_client_configs.clone()
    }
}

impl FactorInstanceBuilder for InstanceBuilder {
    type InstanceState = ();

    fn build(self) -> anyhow::Result<Self::InstanceState> {
        Ok(())
    }
}

/// Records the address host, port, and database as fields on the current tracing span.
///
/// This should only be called from within a function that has been instrumented with a span.
///
/// The following fields must be pre-declared as empty on the span or they will not show up.
/// ```
/// use tracing::field::Empty;
/// #[tracing::instrument(fields(db.address = Empty, server.port = Empty, db.namespace = Empty))]
/// fn open() {}
/// ```
pub fn record_address_fields(address: &str) {
    if let Ok(url) = Url::parse(address) {
        let span = tracing::Span::current();
        // `db.address` and `db.namespace` are incubating in opentelemetry-semantic-conventions 0.28.
        // Leaving as string literals to avoid enabling the semconv_experimental feature.
        span.record("db.address", url.host_str().unwrap_or_default());
        span.record(SERVER_PORT, url.port().unwrap_or_default());
        span.record("db.namespace", url.path().trim_start_matches('/'));
    }
}
