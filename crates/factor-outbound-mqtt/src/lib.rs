mod allowed_hosts;
mod host;

use std::sync::Arc;
use std::time::Duration;

use host::InstanceState;
use rumqttc::{AsyncClient, Event, Incoming, Outgoing, QoS};
use spin_core::async_trait;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factors::{
    ConfigureAppContext, Factor, FactorData, PrepareContext, RuntimeFactors, SelfInstanceBuilder,
};
use spin_world::spin::mqtt::mqtt as v3;
use spin_world::v2::mqtt as v2;
use tokio::sync::Mutex;

pub use host::MqttClient;

use crate::host::other_error_v3;

pub struct OutboundMqttFactor {
    create_client: Arc<dyn ClientCreator>,
}

impl OutboundMqttFactor {
    pub fn new(create_client: Arc<dyn ClientCreator>) -> Self {
        Self { create_client }
    }
}

impl Factor for OutboundMqttFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceState;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(v2::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v3::add_to_linker::<_, MqttFactorData>)?;
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
        let otel = OtelFactorState::from_prepare_context(&mut ctx)?;

        Ok(InstanceState::new(
            allowed_hosts,
            self.create_client.clone(),
            otel,
        ))
    }
}

impl SelfInstanceBuilder for InstanceState {}

struct MqttFactorData;

impl spin_core::wasmtime::component::HasData for MqttFactorData {
    type Data<'a> = &'a mut InstanceState;
}

// This is a concrete implementation of the MQTT client using rumqttc.
pub struct NetworkedMqttClient {
    inner: rumqttc::AsyncClient,
    event_loop: Mutex<rumqttc::EventLoop>,
}

const MQTT_CHANNEL_CAP: usize = 1000;

impl NetworkedMqttClient {
    /// Create a [`ClientCreator`] that creates a [`NetworkedMqttClient`].
    pub fn creator() -> Arc<dyn ClientCreator> {
        Arc::new(|address, username, password, keep_alive_interval| {
            Ok(Arc::new(NetworkedMqttClient::create(
                address,
                username,
                password,
                keep_alive_interval,
            )?) as _)
        })
    }

    /// Create a new [`NetworkedMqttClient`] with the given address, username, password, and keep alive interval.
    pub fn create(
        address: String,
        username: String,
        password: String,
        keep_alive_interval: Duration,
    ) -> Result<Self, v3::Error> {
        let mut conn_opts = rumqttc::MqttOptions::parse_url(address).map_err(|e| {
            tracing::error!("MQTT URL parse error: {e:?}");
            v3::Error::InvalidAddress
        })?;
        conn_opts.set_credentials(username, password);
        conn_opts.set_keep_alive(keep_alive_interval);
        let (client, event_loop) = AsyncClient::new(conn_opts, MQTT_CHANNEL_CAP);
        Ok(Self {
            inner: client,
            event_loop: Mutex::new(event_loop),
        })
    }
}

#[async_trait]
impl MqttClient for NetworkedMqttClient {
    async fn publish_bytes(
        &self,
        topic: String,
        qos: v3::Qos,
        payload: Vec<u8>,
    ) -> Result<(), v3::Error> {
        let qos = match qos {
            v3::Qos::AtMostOnce => rumqttc::QoS::AtMostOnce,
            v3::Qos::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
            v3::Qos::ExactlyOnce => rumqttc::QoS::ExactlyOnce,
        };
        // Message published to EventLoop (not MQTT Broker)
        self.inner
            .publish_bytes(topic, qos, false, payload.into())
            .await
            .map_err(other_error_v3)?;

        // Poll event loop until outgoing publish event is iterated over to send the message to MQTT broker or capture/throw error.
        // We may revisit this later to manage long running connections, high throughput use cases and their issues in the connection pool.
        let mut lock = self.event_loop.lock().await;
        loop {
            let event = lock
                .poll()
                .await
                .map_err(|err| v3::Error::ConnectionFailed(err.to_string()))?;

            match (qos, event) {
                (QoS::AtMostOnce, Event::Outgoing(Outgoing::Publish(_)))
                | (QoS::AtLeastOnce, Event::Incoming(Incoming::PubAck(_)))
                | (QoS::ExactlyOnce, Event::Incoming(Incoming::PubComp(_))) => break,

                (_, _) => continue,
            }
        }
        Ok(())
    }
}

/// A trait for creating MQTT client.
#[async_trait]
pub trait ClientCreator: Send + Sync {
    fn create(
        &self,
        address: String,
        username: String,
        password: String,
        keep_alive_interval: Duration,
    ) -> Result<Arc<dyn MqttClient>, v3::Error>;
}

impl<F> ClientCreator for F
where
    F: Fn(String, String, String, Duration) -> Result<Arc<dyn MqttClient>, v3::Error> + Send + Sync,
{
    fn create(
        &self,
        address: String,
        username: String,
        password: String,
        keep_alive_interval: Duration,
    ) -> Result<Arc<dyn MqttClient>, v3::Error> {
        self(address, username, password, keep_alive_interval)
    }
}
