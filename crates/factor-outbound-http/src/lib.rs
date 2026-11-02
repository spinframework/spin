pub mod intercept;
pub mod runtime_config;
mod spin;
mod wasi;
pub mod wasi_2023_10_18;
pub mod wasi_2023_11_10;
pub mod wasi_2026_03_15;

use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::Context;
use http::{
    HeaderValue, Uri,
    uri::{Authority, Parts, PathAndQuery, Scheme},
};
use intercept::OutboundHttpInterceptor;
use runtime_config::RuntimeConfig;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::{
    ComponentTlsClientConfigs, ConnectionSemaphore, OutboundNetworkingFactor,
    build_connection_semaphore,
    config::{allowed_hosts::OutboundAllowedHosts, blocked_networks::BlockedNetworks},
};
use spin_factors::{
    ConfigureAppContext, Factor, FactorData, FactorField, PrepareContext, RuntimeFactors,
    SelfInstanceBuilder, anyhow,
};
use spin_world::CapabilitySetKey;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView};

pub use wasmtime_wasi_http::p2::{
    HttpResult, bindings::http::types::ErrorCode, body::HyperOutgoingBody,
    types::HostFutureIncomingResponse,
};

#[derive(Default)]
pub struct OutboundHttpFactor {
    cap_set_id: std::sync::RwLock<HashMap<CapabilitySetKey, wasmtime_wasi::NamedId>>,
    id_to_csk: Arc<std::sync::RwLock<Vec<CapabilitySetKey>>>,
    _priv: (),
}

impl Factor for OutboundHttpFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceState;

    fn init<T: spin_factors::InitContext<Self>>(&mut self, ctx: &mut T) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::http::add_to_linker::<_, FactorData<Self>>)?;
        wasi::add_to_linker(ctx)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let config = ctx.take_runtime_config().unwrap_or_default();
        let networking = ctx.app_state::<OutboundNetworkingFactor>().ok();

        for component in ctx.app().components() {
            for caps in component.locked.dependency_capabilities() {
                self.register_capability_set(&caps.capabilities_key)?;
            }
        }

        Ok(AppState {
            wasi_http_clients: wasi::HttpClients::new(config.connection_pooling_enabled),
            connection_pooling_enabled: config.connection_pooling_enabled,
            semaphore: build_connection_semaphore(
                networking,
                "http",
                config.max_concurrent_connections,
                config.wait_timeout,
            ),
        })
    }

    fn register_named_imports<T: spin_factors::InitContext<Self>>(
        &self,
        ctx: &mut T,
        component: &wasmtime::component::Component,
    ) -> anyhow::Result<()> {
        use wasmtime_wasi_http::{p2, p3};

        p2::bindings::named_imports::wasi::http::outgoing_handler::add_to_linker::<
            _,
            wasmtime_wasi_http::WasiHttpNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;
        p2::bindings::named_imports::wasi::http::types::add_to_linker::<
            _,
            wasmtime_wasi_http::WasiHttpNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            &Default::default(),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;

        p3::bindings::named_imports::wasi::http::client::add_to_linker::<
            _,
            wasmtime_wasi_http::WasiHttpNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;
        p3::bindings::named_imports::wasi::http::types::add_to_linker::<
            _,
            wasmtime_wasi_http::WasiHttpNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;

        Ok(())
        // wasi::add_to_linker_named(ctx, component)
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        mut ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let outbound_networking = ctx.instance_builder::<OutboundNetworkingFactor>()?;
        let allowed_hosts = outbound_networking.allowed_hosts();
        let blocked_networks = outbound_networking.blocked_networks();
        let component_tls_configs = outbound_networking.component_tls_configs();
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;

        let mut dependency_ctx = std::collections::HashMap::new();
        let mut dependency_hooks = std::collections::HashMap::new();

        for caps in ctx.app_component().locked.dependency_capabilities() {
            dependency_ctx.insert(
                self.named_id_of(&caps.capabilities_key)?,
                WasiHttpCtx::new(),
            );
            dependency_hooks.insert(
                self.named_id_of(&caps.capabilities_key)?,
                InstanceHttpHooks {
                    capability_set: Some(caps.capabilities_key.clone()),
                    allowed_hosts: allowed_hosts.clone(),
                    blocked_networks: blocked_networks.clone(),
                    component_tls_configs: component_tls_configs.clone(),
                    self_request_origin: None,
                    request_interceptor: None,
                    spin_http_client: None,
                    wasi_http_clients: ctx.app_state().wasi_http_clients.clone(),
                    connection_pooling_enabled: ctx.app_state().connection_pooling_enabled,
                    semaphore: ctx.app_state().semaphore.clone(),
                    otel: otel.clone(),
                },
            );
        }

        Ok(InstanceState {
            wasi_http_ctx: WasiHttpCtx::new(),
            hooks: InstanceHttpHooks {
                capability_set: None,
                allowed_hosts,
                blocked_networks,
                component_tls_configs,
                self_request_origin: None,
                request_interceptor: None,
                spin_http_client: None,
                wasi_http_clients: ctx.app_state().wasi_http_clients.clone(),
                connection_pooling_enabled: ctx.app_state().connection_pooling_enabled,
                semaphore: ctx.app_state().semaphore.clone(),
                otel,
            },
            dependency_ctx,
            dependency_hooks,
        })
    }
}

// TODO: dupe of WasiFactor
impl OutboundHttpFactor {
    fn lookup_named_import(
        &self,
        named_import_name: &str,
    ) -> wasmtime::Result<wasmtime_wasi::NamedId> {
        let named_import_key = spin_world::NamedImportKey::try_from(named_import_name)
            .map_err(wasmtime::Error::from_anyhow)?;
        self.named_id_of(named_import_key.capability_set())
    }

    fn named_id_of(&self, csk: &CapabilitySetKey) -> wasmtime::Result<wasmtime_wasi::NamedId> {
        let named_id_opt = self
            .cap_set_id
            .read()
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?
            .get(csk)
            .cloned();
        named_id_opt.ok_or_else(|| wasmtime::Error::msg("named import ID not found"))
    }

    fn register_capability_set(
        &self,
        cap_set_key: &CapabilitySetKey,
    ) -> wasmtime::Result<wasmtime_wasi::NamedId> {
        if let Ok(id) = self.named_id_of(cap_set_key) {
            return Ok(id);
        }
        // TODO: maybe this should be a single helper struct
        let mut cap_set_id = self
            .cap_set_id
            .write()
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let mut id_to_csk = self
            .id_to_csk
            .write()
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let id = wasmtime_wasi::NamedId(id_to_csk.len());
        id_to_csk.push(cap_set_key.clone());
        cap_set_id.insert(cap_set_key.clone(), id);
        Ok(id)
    }
}

pub struct InstanceState {
    wasi_http_ctx: WasiHttpCtx,
    hooks: InstanceHttpHooks,
    dependency_ctx: std::collections::HashMap<wasmtime_wasi::NamedId, WasiHttpCtx>,
    dependency_hooks: std::collections::HashMap<wasmtime_wasi::NamedId, InstanceHttpHooks>,
}

struct InstanceHttpHooks {
    capability_set: Option<CapabilitySetKey>,
    allowed_hosts: OutboundAllowedHosts,
    blocked_networks: BlockedNetworks,
    component_tls_configs: ComponentTlsClientConfigs,
    self_request_origin: Option<SelfRequestOrigin>,
    request_interceptor: Option<Arc<dyn OutboundHttpInterceptor>>,
    // Connection-pooling client for 'fermyon:spin/http' interface
    //
    // TODO: We could move this to `AppState` like the
    // `wasi:http/outgoing-handler` pool for consistency, although it's probably
    // not a high priority given that `fermyon:spin/http` is deprecated anyway.
    spin_http_client: Option<reqwest::Client>,
    // Connection pooling clients for `wasi:http/outgoing-handler` interface
    //
    // This is a clone of `AppState::wasi_http_clients`, meaning it is shared
    // among all instances of the app.
    wasi_http_clients: wasi::HttpClients,
    /// Whether connection pooling is enabled for this instance.
    connection_pooling_enabled: bool,
    /// Semaphore to limit concurrent outbound connections.
    semaphore: ConnectionSemaphore,
    /// Manages access to the OtelFactor state.
    otel: OtelFactorState,
}

impl InstanceState {
    /// Sets the [`SelfRequestOrigin`] for this instance.
    ///
    /// This is used to handle outbound requests to relative URLs. If unset,
    /// those requests will fail.
    pub fn set_self_request_origin(&mut self, origin: SelfRequestOrigin) {
        self.hooks.self_request_origin = Some(origin.clone());
        for hooks in self.dependency_hooks.values_mut() {
            hooks.self_request_origin = Some(origin.clone());
        }
    }

    /// Sets a [`OutboundHttpInterceptor`] for this instance.
    ///
    /// Returns an error if it has already been called for this instance.
    pub fn set_request_interceptor(
        &mut self,
        interceptor: impl OutboundHttpInterceptor + 'static,
    ) -> anyhow::Result<()> {
        if self.hooks.request_interceptor.is_some() {
            anyhow::bail!("set_request_interceptor can only be called once");
        }

        let interceptor = Arc::new(interceptor);
        self.hooks.request_interceptor = Some(interceptor.clone());
        for hooks in self.dependency_hooks.values_mut() {
            hooks.request_interceptor = Some(interceptor.clone());
        }
        Ok(())
    }
}

impl SelfInstanceBuilder for InstanceState {}

pub type Request = http::Request<wasmtime_wasi_http::p2::body::HyperOutgoingBody>;
pub type Response = http::Response<wasmtime_wasi_http::p2::body::HyperIncomingBody>;

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
    /// Whether connection pooling is enabled for this app.
    connection_pooling_enabled: bool,
    /// Semaphore to limit concurrent outbound connections.
    semaphore: ConnectionSemaphore,
}

/// Removes IPs in the given [`BlockedNetworks`].
///
/// Returns [`ErrorCode::DestinationIpProhibited`] if all IPs are removed.
fn remove_blocked_addrs(
    blocked_networks: &BlockedNetworks,
    addrs: &mut Vec<SocketAddr>,
) -> Result<(), ErrorCode> {
    if addrs.is_empty() {
        return Ok(());
    }
    let blocked_addrs = blocked_networks.remove_blocked(addrs);
    if addrs.is_empty() && !blocked_addrs.is_empty() {
        tracing::error!(
            "error.type" = "destination_ip_prohibited",
            ?blocked_addrs,
            "all destination IP(s) prohibited by runtime config"
        );
        return Err(ErrorCode::DestinationIpProhibited);
    }
    Ok(())
}

#[repr(transparent)]
struct WasiStoreDataWrapper<T>
where
    T: FactorField<Factor = OutboundHttpFactor>,
{
    data: T::State,
}

impl<T> WasiStoreDataWrapper<T>
where
    T: FactorField<Factor = OutboundHttpFactor>,
{
    fn from_mut(data: &mut T::State) -> &mut Self {
        // SAFETY: TODO (aka repr(transparent), one field, etc)
        unsafe { &mut *(data as *mut T::State as *mut Self) }
    }
}

impl<T> wasmtime_wasi_http::WasiHttpNamedView for WasiStoreDataWrapper<T>
where
    T: FactorField<Factor = OutboundHttpFactor>,
{
    fn http(&mut self, id: wasmtime_wasi::NamedId) -> WasiHttpCtxView<'_> {
        let (st, table) = T::get(&mut self.data);
        WasiHttpCtxView {
            hooks: st.dependency_hooks.get_mut(&id).unwrap(),
            table,
            ctx: st.dependency_ctx.get_mut(&id).unwrap(),
        }
    }
}
