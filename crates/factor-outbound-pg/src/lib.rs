mod allowed_hosts;
pub mod client;
mod host;
pub mod runtime_config;
mod types;

use std::collections::HashMap;
use std::sync::Arc;

use allowed_hosts::AllowedHostChecker;
use client::ClientFactory;
use runtime_config::RuntimeConfig;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::{
    ConnectionSemaphore, OutboundNetworkingFactor, build_connection_semaphore,
};
use spin_factors::{
    ConfigureAppContext, Factor, PrepareContext, RuntimeFactors, SelfInstanceBuilder, anyhow,
};

pub struct OutboundPgFactor<CF = crate::client::PooledTokioClientFactory> {
    _phantom: std::marker::PhantomData<CF>,
}

pub struct AppState<CF> {
    pub client_factories: HashMap<String, Arc<CF>>,
    /// Semaphore to limit concurrent outbound PostgreSQL connections.
    pub semaphore: ConnectionSemaphore,
}

impl<CF: ClientFactory> Factor for OutboundPgFactor<CF> {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState<CF>;
    type InstanceBuilder = InstanceState<CF>;

    fn init<T: spin_factors::InitContext<Self>>(&mut self, ctx: &mut T) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::postgres::add_to_linker::<_, PgFactorData<CF>>)?;
        ctx.link_bindings(spin_world::v2::postgres::add_to_linker::<_, PgFactorData<CF>>)?;
        ctx.link_bindings(
            spin_world::spin::postgres3_0_0::postgres::add_to_linker::<_, PgFactorData<CF>>,
        )?;
        ctx.link_bindings(
            spin_world::spin::postgres4_2_0::postgres::add_to_linker::<_, PgFactorData<CF>>,
        )?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let config = ctx.take_runtime_config().unwrap_or_default();
        let mut client_factories = HashMap::new();
        for comp in ctx.app().components() {
            client_factories.insert(comp.id().to_string(), Arc::new(CF::default()));
        }
        let networking = ctx.app_state::<OutboundNetworkingFactor>().ok();

        Ok(AppState {
            client_factories,
            semaphore: build_connection_semaphore(
                networking,
                "pg",
                config.max_connections,
                config.wait_timeout,
            ),
        })
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        mut ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let allowed_hosts = ctx
            .instance_builder::<OutboundNetworkingFactor>()?
            .allowed_hosts();
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;
        let cf = ctx
            .app_state()
            .client_factories
            .get(ctx.app_component().id())
            .unwrap();

        Ok(InstanceState {
            allowed_host_checker: AllowedHostChecker::new(allowed_hosts),
            client_factory: cf.clone(),
            connections: Default::default(),
            otel,
            builders: Default::default(),
            semaphore: ctx.app_state().semaphore.clone(),
        })
    }
}

impl<C> Default for OutboundPgFactor<C> {
    fn default() -> Self {
        Self {
            _phantom: Default::default(),
        }
    }
}

impl<C> OutboundPgFactor<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct InstanceState<CF: ClientFactory> {
    allowed_host_checker: AllowedHostChecker,
    client_factory: Arc<CF>,
    connections: spin_resource_table::Table<(
        CF::Client,
        spin_factor_outbound_networking::ConnectionPermit,
    )>,
    otel: OtelFactorState,
    builders: spin_resource_table::Table<host::ConnectionBuilder>,
    pub semaphore: ConnectionSemaphore,
}

impl<CF: ClientFactory> SelfInstanceBuilder for InstanceState<CF> {}

pub struct PgFactorData<CF: ClientFactory>(OutboundPgFactor<CF>);

impl<CF: ClientFactory> spin_core::wasmtime::component::HasData for PgFactorData<CF> {
    type Data<'a> = &'a mut InstanceState<CF>;
}

impl<CF: ClientFactory> spin_core::wasmtime::component::HasData for InstanceState<CF> {
    type Data<'a> = &'a mut InstanceState<CF>;
}
