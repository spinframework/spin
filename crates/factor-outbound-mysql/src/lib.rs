pub mod client;
mod host;

use client::Client;
use mysql_async::Conn as MysqlClient;
use spin_factor_otel::OtelContext;
use spin_factor_outbound_networking::{
    config::allowed_hosts::OutboundAllowedHosts, OutboundNetworkingFactor,
};
use spin_factors::{Factor, FactorData, InitContext, RuntimeFactors, SelfInstanceBuilder};
use spin_world::v1::mysql as v1;
use spin_world::v2::mysql::{self as v2};

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
        let otel_context = OtelContext::from_prepare_context(&mut ctx)?;

        Ok(InstanceState {
            allowed_hosts,
            connections: Default::default(),
            otel_context,
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

pub struct InstanceState<C> {
    allowed_hosts: OutboundAllowedHosts,
    connections: spin_resource_table::Table<C>,
    otel_context: OtelContext,
}

impl<C: Send + 'static> SelfInstanceBuilder for InstanceState<C> {}
