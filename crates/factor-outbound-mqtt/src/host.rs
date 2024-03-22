use std::{sync::Arc, time::Duration};

use anyhow::Result;
use spin_core::{async_trait, wasmtime::component::Resource};
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::config::allowed_hosts::OutboundAllowedHosts;
use spin_world::v2::mqtt::{self as v2, Connection, Error, Qos};
use tracing::{instrument, Level};

use crate::ClientCreator;

pub struct InstanceState {
    allowed_hosts: OutboundAllowedHosts,
    connections: spin_resource_table::Table<Arc<dyn MqttClient>>,
    create_client: Arc<dyn ClientCreator>,
    otel: OtelFactorState,
}

impl InstanceState {
    pub fn new(
        allowed_hosts: OutboundAllowedHosts,
        create_client: Arc<dyn ClientCreator>,
        otel: OtelFactorState,
    ) -> Self {
        Self {
            allowed_hosts,
            create_client,
            connections: spin_resource_table::Table::new(1024),
            otel,
        }
    }
}

#[async_trait]
pub trait MqttClient: Send + Sync {
    async fn publish_bytes(&self, topic: String, qos: Qos, payload: Vec<u8>) -> Result<(), Error>;
}

impl InstanceState {
    async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        self.allowed_hosts.check_url(address, "mqtt").await
    }

    async fn establish_connection(
        &mut self,
        address: String,
        username: String,
        password: String,
        keep_alive_interval: Duration,
    ) -> Result<Resource<Connection>, Error> {
        self.connections
            .push((self.create_client).create(address, username, password, keep_alive_interval)?)
            .map(Resource::new_own)
            .map_err(|_| Error::TooManyConnections)
    }

    async fn get_conn(&self, connection: Resource<Connection>) -> Result<&dyn MqttClient, Error> {
        self.connections
            .get(connection.rep())
            .ok_or(Error::Other(
                "could not find connection for resource".into(),
            ))
            .map(|c| c.as_ref())
    }
}

impl v2::Host for InstanceState {
    fn convert_error(&mut self, error: Error) -> Result<Error> {
        Ok(error)
    }
}

impl v2::HostConnection for InstanceState {
    #[instrument(name = "spin_outbound_mqtt.open_connection", skip(self, password), err(level = Level::INFO), fields(otel.kind = "client"))]
    async fn open(
        &mut self,
        address: String,
        username: String,
        password: String,
        keep_alive_interval: u64,
    ) -> Result<Resource<Connection>, Error> {
        self.otel.reparent_tracing_span();

        if !self
            .is_address_allowed(&address)
            .await
            .map_err(|e| v2::Error::Other(e.to_string()))?
        {
            return Err(v2::Error::ConnectionFailed(format!(
                "address {address} is not permitted"
            )));
        }
        self.establish_connection(
            address,
            username,
            password,
            Duration::from_secs(keep_alive_interval),
        )
        .await
    }

    /// Publish a message to the MQTT broker.
    ///
    /// OTEL trace propagation is not directly supported in MQTT V3. You will need to embed the
    /// current trace context into the payload yourself.
    /// https://w3c.github.io/trace-context-mqtt/#mqtt-v3-recommendation.
    #[instrument(name = "spin_outbound_mqtt.publish", skip(self, connection, payload), err(level = Level::INFO),
        fields(otel.kind = "producer", otel.name = format!("{} publish", topic), messaging.operation = "publish",
        messaging.system = "mqtt"))]
    async fn publish(
        &mut self,
        connection: Resource<Connection>,
        topic: String,
        payload: Vec<u8>,
        qos: Qos,
    ) -> Result<(), Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection).await.map_err(other_error)?;

        conn.publish_bytes(topic, qos, payload).await?;

        Ok(())
    }

    async fn drop(&mut self, connection: Resource<Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

pub fn other_error(e: impl std::fmt::Display) -> Error {
    Error::Other(e.to_string())
}
