pub mod client;
mod host;

use client::Client;
use mysql_async::Conn as MysqlClient;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::{
    OutboundNetworkingFactor, config::allowed_hosts::OutboundAllowedHosts,
};
use spin_factors::{Factor, FactorData, InitContext, RuntimeFactors, SelfInstanceBuilder};
use spin_world::spin::mysql::mysql as v3;
use spin_world::v1::mysql as v1;
use spin_world::v2::mysql as v2;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct OutboundMysqlFactor<C = MysqlClient> {
    _phantom: std::marker::PhantomData<C>,
}

impl<C: Send + Sync + Client + 'static> Factor for OutboundMysqlFactor<C> {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceState<C>;

    fn init(&mut self, ctx: &mut impl InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(v1::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v2::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v3::add_to_linker::<_, MysqlFactorData<C>>)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        _ctx: spin_factors::ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        Ok(())
    }

    fn prepare<T: spin_factors::RuntimeFactors>(
        &self,
        mut ctx: spin_factors::PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let allowed_hosts = ctx
            .instance_builder::<OutboundNetworkingFactor>()?
            .allowed_hosts();
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;

        Ok(InstanceState(Arc::new(Mutex::new(InstanceStateInner {
            allowed_hosts,
            connections: Default::default(),
            otel,
        }))))
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
    connections: spin_resource_table::Table<Arc<Mutex<C>>>,
    otel: OtelFactorState,
}

pub struct InstanceState<C>(Arc<Mutex<InstanceStateInner<C>>>);

impl<C: Send + 'static> SelfInstanceBuilder for InstanceState<C> {}

pub struct MysqlFactorData<C: Client>(OutboundMysqlFactor<C>);

impl<C: Client> spin_core::wasmtime::component::HasData for MysqlFactorData<C> {
    type Data<'a> = &'a mut InstanceState<C>;
}

impl<C: Client> spin_core::wasmtime::component::HasData for InstanceState<C> {
    type Data<'a> = &'a mut InstanceState<C>;
}
