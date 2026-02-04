pub mod client;
mod host;
mod types;

use std::{collections::HashMap, sync::Arc};

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
    type AppState = HashMap<String, Arc<CF>>;
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
        ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let mut client_factories = HashMap::new();
        for comp in ctx.app().components() {
            client_factories.insert(comp.id().to_string(), Arc::new(CF::default()));
        }
        Ok(client_factories)
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        mut ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let outbound_networking = ctx.instance_builder::<OutboundNetworkingFactor>()?;
        let allowed_hosts = outbound_networking.allowed_hosts();
        let cf = ctx.app_state().get(ctx.app_component().id()).unwrap();
        let assets = ctx.app_component().files().cloned().collect();

        Ok(InstanceState {
            allowed_hosts,
            client_factory: cf.clone(),
            connections: Default::default(),
            assets,
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
    assets: Vec<spin_locked_app::locked::ContentPath>,
}

impl<CF: ClientFactory> SelfInstanceBuilder for InstanceState<CF> {}
