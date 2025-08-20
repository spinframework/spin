pub mod intercept;
pub mod runtime_config;
mod spin;
mod wasi;
pub mod wasi_2023_10_18;
pub mod wasi_2023_11_10;

use std::sync::Arc;

use anyhow::Context;
use http::{
    uri::{Authority, Parts, PathAndQuery, Scheme},
    HeaderValue, Uri,
};
use intercept::OutboundHttpInterceptor;
use runtime_config::RuntimeConfig;
use spin_factor_outbound_networking::{
    config::{allowed_hosts::OutboundAllowedHosts, blocked_networks::BlockedNetworks},
    ComponentTlsClientConfigs, OutboundNetworkingFactor,
};
use spin_factors::{
    anyhow, ConfigureAppContext, Factor, FactorData, PrepareContext, RuntimeFactors,
    SelfInstanceBuilder,
};
use wasmtime_wasi_http::WasiHttpCtx;

pub use wasmtime_wasi_http::{
    body::HyperOutgoingBody,
    types::{HostFutureIncomingResponse, OutgoingRequestConfig},
    HttpResult,
};

#[derive(Default)]
pub struct OutboundHttpFactor {
    _priv: (),
}

impl Factor for OutboundHttpFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceState;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::http::add_to_linker::<_, FactorData<Self>>)?;
        wasi::add_to_linker(ctx)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let connection_pooling = ctx
            .take_runtime_config()
            .unwrap_or_default()
            .connection_pooling;
        Ok(AppState {
            wasi_http_clients: wasi::HttpClients::new(connection_pooling),
            connection_pooling,
        })
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        mut ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let outbound_networking = ctx.instance_builder::<OutboundNetworkingFactor>()?;
        let allowed_hosts = outbound_networking.allowed_hosts();
        let blocked_networks = outbound_networking.blocked_networks();
        let component_tls_configs = outbound_networking.component_tls_configs();
        Ok(InstanceState {
            wasi_http_ctx: WasiHttpCtx::new(),
            allowed_hosts,
            blocked_networks,
            component_tls_configs,
            self_request_origin: None,
            request_interceptor: None,
            spin_http_client: None,
            wasi_http_clients: ctx.app_state().wasi_http_clients.clone(),
            connection_pooling: ctx.app_state().connection_pooling,
        })
    }
}

pub struct InstanceState {
    wasi_http_ctx: WasiHttpCtx,
    allowed_hosts: OutboundAllowedHosts,
    blocked_networks: BlockedNetworks,
    component_tls_configs: ComponentTlsClientConfigs,
    self_request_origin: Option<SelfRequestOrigin>,
    request_interceptor: Option<Arc<dyn OutboundHttpInterceptor>>,
    // Connection-pooling client for 'fermyon:spin/http' interface
    //
    // TODO: We could move this to `AppState` to like the
    // `wasi:http/outgoing-handler` pool for consistency, although it's probably
    // not a high priority given that `fermyon:spin/http` is deprecated anyway.
    spin_http_client: Option<reqwest::Client>,
    // Connection pooling clients for `wasi:http/outgoing-handler` interface
    //
    // This is a clone of `AppState::wasi_http_clients`, meaning it is shared
    // among all instances of the app.
    wasi_http_clients: wasi::HttpClients,
    connection_pooling: bool,
}

impl InstanceState {
    /// Sets the [`SelfRequestOrigin`] for this instance.
    ///
    /// This is used to handle outbound requests to relative URLs. If unset,
    /// those requests will fail.
    pub fn set_self_request_origin(&mut self, origin: SelfRequestOrigin) {
        self.self_request_origin = Some(origin);
    }

    /// Sets a [`OutboundHttpInterceptor`] for this instance.
    ///
    /// Returns an error if it has already been called for this instance.
    pub fn set_request_interceptor(
        &mut self,
        interceptor: impl OutboundHttpInterceptor + 'static,
    ) -> anyhow::Result<()> {
        if self.request_interceptor.is_some() {
            anyhow::bail!("set_request_interceptor can only be called once");
        }
        self.request_interceptor = Some(Arc::new(interceptor));
        Ok(())
    }
}

impl SelfInstanceBuilder for InstanceState {}

pub type Request = http::Request<wasmtime_wasi_http::body::HyperOutgoingBody>;
pub type Response = http::Response<wasmtime_wasi_http::body::HyperIncomingBody>;

/// SelfRequestOrigin indicates the base URI to use for "self" requests.
#[derive(Clone, Debug)]
pub struct SelfRequestOrigin {
    pub scheme: Scheme,
    pub authority: Authority,
}

impl SelfRequestOrigin {
    pub fn create(scheme: Scheme, auth: &str) -> anyhow::Result<Self> {
        Ok(SelfRequestOrigin {
            scheme,
            authority: auth
                .parse()
                .with_context(|| format!("address '{auth}' is not a valid authority"))?,
        })
    }

    pub fn from_uri(uri: &Uri) -> anyhow::Result<Self> {
        Ok(Self {
            scheme: uri.scheme().context("URI missing scheme")?.clone(),
            authority: uri.authority().context("URI missing authority")?.clone(),
        })
    }

    fn into_uri(self, path_and_query: Option<PathAndQuery>) -> Uri {
        let mut parts = Parts::default();
        parts.scheme = Some(self.scheme);
        parts.authority = Some(self.authority);
        parts.path_and_query = path_and_query;
        Uri::from_parts(parts).unwrap()
    }

    fn use_tls(&self) -> bool {
        self.scheme == Scheme::HTTPS
    }

    fn host_header(&self) -> HeaderValue {
        HeaderValue::from_str(self.authority.as_str()).unwrap()
    }
}

impl std::fmt::Display for SelfRequestOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}", self.scheme, self.authority)
    }
}

pub struct AppState {
    // Connection pooling clients for `wasi:http/outgoing-handler` interface
    wasi_http_clients: wasi::HttpClients,
    connection_pooling: bool,
}
