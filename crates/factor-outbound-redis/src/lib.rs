mod allowed_hosts;
mod host;

use host::InstanceState;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factors::{
    ConfigureAppContext, Factor, FactorData, PrepareContext, RuntimeFactors, SelfInstanceBuilder,
    anyhow,
};
use spin_world::spin::redis::redis as v3;

use crate::allowed_hosts::AllowedHostChecker;

/// The [`Factor`] for `fermyon:spin/outbound-redis`.
#[derive(Default)]
pub struct OutboundRedisFactor {
    _priv: (),
}

impl OutboundRedisFactor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Factor for OutboundRedisFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceState;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::v1::redis::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(spin_world::v2::redis::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v3::add_to_linker::<_, RedisFactorData>)?;
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
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;
        let outbound_networking = ctx.instance_builder::<OutboundNetworkingFactor>()?;

        Ok(InstanceState {
            allowed_host_checker: AllowedHostChecker::new(outbound_networking.allowed_hosts()),
            blocked_networks: outbound_networking.blocked_networks(),
            connections: spin_resource_table::Table::new(1024),
            otel,
        })
    }
}

impl SelfInstanceBuilder for InstanceState {}

pub struct RedisFactorData(OutboundRedisFactor);

impl spin_core::wasmtime::component::HasData for RedisFactorData {
    type Data<'a> = &'a mut InstanceState;
}
