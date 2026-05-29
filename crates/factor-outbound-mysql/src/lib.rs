pub mod client;
mod host;
pub mod runtime_config;

use std::sync::Arc;

use client::Client;
use mysql_async::Conn as MysqlClient;
use runtime_config::RuntimeConfig;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::{
    OutboundNetworkingFactor, config::allowed_hosts::OutboundAllowedHosts,
};
use spin_factors::{Factor, FactorData, InitContext, RuntimeFactors, SelfInstanceBuilder};
use spin_world::spin::mysql::mysql as v3;
use spin_world::v1::mysql as v1;
use spin_world::v2::mysql as v2;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

pub struct OutboundMysqlFactor<C = MysqlClient> {
    _phantom: std::marker::PhantomData<C>,
}

pub struct AppState {
    /// A semaphore to limit the number of concurrent outbound MySQL connections.
    pub connection_semaphore: Option<Arc<Semaphore>>,
}

impl<C: Send + Sync + Client + 'static> Factor for OutboundMysqlFactor<C> {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceState<C>;

    fn init(&mut self, ctx: &mut impl InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(v1::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v2::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v3::add_to_linker::<_, MysqlFactorData<C>>)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: spin_factors::ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let config = ctx.take_runtime_config().unwrap_or_default();
        Ok(AppState {
            connection_semaphore: config.max_connections.map(|n| Arc::new(Semaphore::new(n))),
        })
    }

    fn prepare<T: spin_factors::RuntimeFactors>(
        &self,
        mut ctx: spin_factors::PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let allowed_hosts = ctx
            .instance_builder::<OutboundNetworkingFactor>()?
            .allowed_hosts();
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;

        Ok(InstanceState {
            inner: Arc::new(Mutex::new(InstanceStateInner {
                allowed_hosts,
                connections: Default::default(),
                otel,
            })),
            connection_semaphore: ctx.app_state().connection_semaphore.clone(),
        })
    }
}

impl<C> Default for OutboundMysqlFactor<C> {
    fn default() -> Self {
        Self {
            _phantom: Default::default(),
        }
    }
}

impl<C> OutboundMysqlFactor<C> {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct InstanceStateInner<C> {
    allowed_hosts: OutboundAllowedHosts,
    connections: spin_resource_table::Table<(Arc<Mutex<C>>, Option<OwnedSemaphorePermit>)>,
    otel: OtelFactorState,
}

pub struct InstanceState<C> {
    pub(crate) inner: Arc<Mutex<InstanceStateInner<C>>>,
    pub connection_semaphore: Option<Arc<Semaphore>>,
}

impl<C: Send + 'static> SelfInstanceBuilder for InstanceState<C> {}

pub struct MysqlFactorData<C: Client>(OutboundMysqlFactor<C>);

impl<C: Client> spin_core::wasmtime::component::HasData for MysqlFactorData<C> {
    type Data<'a> = &'a mut InstanceState<C>;
}

impl<C: Client> spin_core::wasmtime::component::HasData for InstanceState<C> {
    type Data<'a> = &'a mut InstanceState<C>;
}
