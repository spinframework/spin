pub mod client;
mod host;

use client::Client;
use spin_factor_outbound_networking::{
    config::allowed_hosts::OutboundAllowedHosts, OutboundNetworkingFactor,
};
use spin_factors::{
    anyhow, ConfigureAppContext, Factor, FactorData, PrepareContext, RuntimeFactors,
    SelfInstanceBuilder,
};
use tokio_postgres::Client as PgClient;

pub struct OutboundPgFactor<C = PgClient> {
    _phantom: std::marker::PhantomData<C>,
}

impl<C: Send + Sync + Client + 'static> Factor for OutboundPgFactor<C> {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceState<C>;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::postgres::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(spin_world::v2::postgres::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(
            spin_world::spin::postgres::postgres::add_to_linker::<_, FactorData<Self>>,
        )?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        _ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        Ok(())
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

pub struct InstanceState<C> {
    allowed_hosts: OutboundAllowedHosts,
    connections: spin_resource_table::Table<C>,
}

impl<C: Send + 'static> SelfInstanceBuilder for InstanceState<C> {}
