pub mod client;
mod host;
mod types;

use std::sync::Arc;

use client::ClientFactory;
use spin_factor_outbound_networking::{
    config::allowed_hosts::OutboundAllowedHosts, OutboundNetworkingFactor,
};
use spin_factors::{
    anyhow, ConfigureAppContext, Factor, FactorData, PrepareContext, RuntimeFactors,
    SelfInstanceBuilder,
};

pub struct OutboundPgFactor<CF = crate::client::PooledTokioClientFactory> {
    _phantom: std::marker::PhantomData<CF>,
}

impl<CF: ClientFactory> Factor for OutboundPgFactor<CF> {
    type RuntimeConfig = ();
    type AppState = Arc<CF>;
    type InstanceBuilder = InstanceState<CF>;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::postgres::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(spin_world::v2::postgres::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(
            spin_world::spin::postgres3_0_0::postgres::add_to_linker::<_, FactorData<Self>>,
        )?;
        ctx.link_bindings(
            spin_world::spin::postgres4_0_0::postgres::add_to_linker::<_, FactorData<Self>>,
        )?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        _ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        Ok(Arc::new(CF::default()))
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        mut ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let allowed_hosts = ctx
            .instance_builder::<OutboundNetworkingFactor>()?
            .allowed_hosts();
        Ok(InstanceState {
            allowed_hosts,
            client_factory: ctx.app_state().clone(),
            connections: Default::default(),
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
    allowed_hosts: OutboundAllowedHosts,
    client_factory: Arc<CF>,
    connections: spin_resource_table::Table<CF::Client>,
}

impl<CF: ClientFactory> SelfInstanceBuilder for InstanceState<CF> {}
