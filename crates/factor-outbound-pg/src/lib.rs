mod allowed_hosts;
pub mod client;
mod host;
mod types;

use std::{collections::HashMap, sync::Arc};

use allowed_hosts::AllowedHostChecker;
use client::ClientFactory;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factors::{
    anyhow, ConfigureAppContext, Factor, PrepareContext, RuntimeFactors, SelfInstanceBuilder,
};

pub struct OutboundPgFactor<CF = crate::client::PooledTokioClientFactory> {
    _phantom: std::marker::PhantomData<CF>,
}

impl<CF: ClientFactory> Factor for OutboundPgFactor<CF> {
    type RuntimeConfig = ();
    type AppState = HashMap<String, Arc<CF>>;
    type InstanceBuilder = InstanceState<CF>;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
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
        let allowed_hosts = ctx
            .instance_builder::<OutboundNetworkingFactor>()?
            .allowed_hosts();
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;
        let cf = ctx.app_state().get(ctx.app_component().id()).unwrap();

        Ok(InstanceState {
            allowed_host_checker: AllowedHostChecker::new(allowed_hosts),
            client_factory: cf.clone(),
            connections: Default::default(),
            otel,
            builders: Default::default(),
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
    connections: spin_resource_table::Table<CF::Client>,
    otel: OtelFactorState,
    builders: spin_resource_table::Table<host::ConnectionBuilder>,
}

impl<CF: ClientFactory> SelfInstanceBuilder for InstanceState<CF> {}

pub struct PgFactorData<CF: ClientFactory>(OutboundPgFactor<CF>);

impl<CF: ClientFactory> spin_core::wasmtime::component::HasData for PgFactorData<CF> {
    type Data<'a> = &'a mut InstanceState<CF>;
}

impl<CF: ClientFactory> spin_core::wasmtime::component::HasData for InstanceState<CF> {
    type Data<'a> = &'a mut InstanceState<CF>;
}
