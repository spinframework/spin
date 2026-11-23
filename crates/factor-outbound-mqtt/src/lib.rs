mod allowed_hosts;
mod host;
pub mod runtime_config;

use std::sync::Arc;
use std::time::Duration;

use host::InstanceState;
use rumqttc::{AsyncClient, Event, Incoming, Outgoing, QoS};
use spin_core::async_trait;
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::{
    ConnectionSemaphore, OutboundNetworkingFactor, build_connection_semaphore,
};
use spin_factors::{
    ConfigureAppContext, Factor, FactorData, PrepareContext, RuntimeFactors, SelfInstanceBuilder,
    anyhow,
};
use spin_world::spin::mqtt::mqtt as v3;
use spin_world::v2::mqtt as v2;
use tokio::sync::Mutex;

pub use host::MqttClient;

use crate::host::other_error_v3;
use crate::runtime_config::RuntimeConfig;

// We cap guest-provided `inflight_num` MQTT options at this value.
// The limit (100) is the same as the MQTT default.
// A future enhancement is to make this runtime-config-able.
const MAX_INFLIGHT: u16 = 100;

pub struct OutboundMqttFactor {
    create_client: Arc<dyn ClientCreator>,
}

impl OutboundMqttFactor {
    pub fn new(create_client: Arc<dyn ClientCreator>) -> Self {
        Self { create_client }
    }
}

pub struct AppState {
    /// Optional maximum payload size in bytes for MQTT messages. If `None`, no limit is enforced.
    max_payload_size_bytes: Option<usize>,
    /// Semaphore to limit concurrent outbound MQTT connections.
    pub semaphore: ConnectionSemaphore,
}

impl Factor for OutboundMqttFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceState;

    fn init<T: spin_factors::InitContext<Self>>(&mut self, ctx: &mut T) -> anyhow::Result<()> {
        ctx.link_bindings(v2::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(v3::add_to_linker::<_, MqttFactorData>)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let config = ctx.take_runtime_config().unwrap_or_default();
        let networking = ctx.app_state::<OutboundNetworkingFactor>().ok();

        Ok(AppState {
            semaphore: build_connection_semaphore(
                networking,
                "mqtt",
                config.max_connections,
                config.wait_timeout,
            ),
            max_payload_size_bytes: config.max_payload_size_bytes,
        })
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
            ctx.app_state().semaphore.clone(),
            otel,
            ctx.app_state().max_payload_size_bytes,
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
        // MqttOptions::parse_url asserts on certain option values. Yes,
        // you read that right. So we have to validate the address beforehand.
        reject_terrible_asserts(&address).map_err(|e| {
            tracing::warn!("invalid MQTT connection string: {e}");
            v3::Error::InvalidAddress
        })?;

        let mut conn_opts = rumqttc::MqttOptions::parse_url(address).map_err(|e| {
            tracing::error!("MQTT URL parse error: {e:?}");
            v3::Error::InvalidAddress
        })?;
        conn_opts.set_credentials(username, password);
        conn_opts.set_keep_alive(keep_alive_interval);
        if conn_opts.inflight() > MAX_INFLIGHT {
            conn_opts.set_inflight(MAX_INFLIGHT);
        }

        let (client, event_loop) = AsyncClient::new(conn_opts, MQTT_CHANNEL_CAP);
        Ok(Self {
            inner: client,
            event_loop: Mutex::new(event_loop),
        })
    }
}

fn reject_terrible_asserts(address: &str) -> Result<(), rumqttc::OptionError> {
    // MQTT's URL parser calls some setters which assert, rather than erroring,
    // on invalid values, but do not check their preconditions. I believe that
    // "inflight_num=0" and "clean_session=false" when client_id is empty are
    // the only settings affected - anything else, including parse errors, does result
    // in a proper error. (Specifically we do not need to check for parse errors,
    // only handle successful parses that would later assert.)
    //
    // The strings in this function are taken from rumqttc::MqttOptions::try_from.

    let url = url::Url::parse(address).map_err(rumqttc::OptionError::Parse)?;
    let queries = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();

    let inflight = queries.get("inflight_num");
    if let Some(inflight) = inflight.and_then(|s| s.parse::<u16>().ok())
        && inflight == 0
    {
        return Err(rumqttc::OptionError::Inflight);
    }

    let client_id = queries.get("client_id");
    let clean_session = queries.get("clean_session");
    if let Some(clean_session) = clean_session.and_then(|s| s.parse::<bool>().ok())
        && client_id.is_some_and(|cid| cid.is_empty())
        && !clean_session
    {
        return Err(rumqttc::OptionError::CleanSession);
    }

    Ok(())
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn terrible_asserts_are_rejected() {
        reject_terrible_asserts("mqtt://h?inflight_num=0").expect_err("inflight 0");
        reject_terrible_asserts("mqtt://h?inflight_num=1").expect("inflight 1");

        reject_terrible_asserts("mqtt://h?client_id=&clean_session=false")
            .expect_err("cid empty, clean_session false");
        reject_terrible_asserts("mqtt://h?client_id=fie&clean_session=false")
            .expect("cid nonempty, clean_session false");
        reject_terrible_asserts("mqtt://h?client_id=&clean_session=true")
            .expect("cid empty, clean_session true");
        reject_terrible_asserts("mqtt://h?client_id=fie&clean_session=true")
            .expect("cid nonempty, clean_session true");
    }
}
