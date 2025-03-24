pub mod client;
mod host;

use std::sync::Arc;

use client::ClientFactory;
use spin_factor_outbound_networking::{OutboundAllowedHosts, OutboundNetworkingFactor};
use spin_factors::{
    anyhow, ConfigureAppContext, Factor, PrepareContext, RuntimeFactors, SelfInstanceBuilder,
};
use tokio::sync::RwLock;

pub struct OutboundPgFactor<CF = crate::client::PooledTokioClientFactory> {
    _phantom: std::marker::PhantomData<CF>,
}

impl<CF: ClientFactory + Send + Sync + 'static> Factor for OutboundPgFactor<CF> {
    type RuntimeConfig = ();
    type AppState = Arc<RwLock<CF>>;
    type InstanceBuilder = InstanceState<CF>;

    fn init<T: Send + 'static>(
        &mut self,
        mut ctx: spin_factors::InitContext<T, Self>,
    ) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::postgres::add_to_linker)?;
        ctx.link_bindings(spin_world::v2::postgres::add_to_linker)?;
        ctx.link_bindings(spin_world::spin::postgres::postgres::add_to_linker)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        _ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        Ok(Arc::new(RwLock::new(CF::new())))
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
    client_factory: Arc<RwLock<CF>>,
    connections: spin_resource_table::Table<CF::Client>,
}

impl<CF: ClientFactory + Send + 'static> SelfInstanceBuilder for InstanceState<CF> {}
