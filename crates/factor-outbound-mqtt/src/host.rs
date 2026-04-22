use std::{sync::Arc, time::Duration};

use anyhow::Result;
use spin_core::{
    async_trait,
    wasmtime::component::{Accessor, Resource},
};
use spin_factor_otel::OtelFactorState;
use spin_factor_outbound_networking::config::allowed_hosts::OutboundAllowedHosts;
use spin_world::spin::mqtt::mqtt as v3;
use spin_world::v2::mqtt as v2;
use tracing::{Level, instrument};

use crate::{ClientCreator, allowed_hosts::AllowedHostChecker};

pub struct InstanceState {
    allowed_hosts: AllowedHostChecker,
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
            allowed_hosts: AllowedHostChecker::new(allowed_hosts),
            create_client,
            connections: spin_resource_table::Table::new(1024),
            otel,
        }
    }
}

#[async_trait]
pub trait MqttClient: Send + Sync {
    async fn publish_bytes(
        &self,
        topic: String,
        qos: v3::Qos,
        payload: Vec<u8>,
    ) -> Result<(), v3::Error>;
}

impl InstanceState {
    async fn is_address_allowed(&self, address: &str) -> Result<bool> {
        self.allowed_hosts.is_address_allowed(address).await
    }

    async fn establish_connection(
        &mut self,
        address: String,
        username: String,
        password: String,
        keep_alive_interval: Duration,
    ) -> Result<Resource<v2::Connection>, v2::Error> {
        self.connections
            .push((self.create_client).create(address, username, password, keep_alive_interval)?)
            .map(Resource::new_own)
            .map_err(|_| v2::Error::TooManyConnections)
    }

    fn get_conn(&self, connection: Resource<v2::Connection>) -> Result<&dyn MqttClient, v2::Error> {
        self.connections
            .get(connection.rep())
            .ok_or(v2::Error::Other(
                "could not find connection for resource".into(),
            ))
            .map(|c| c.as_ref())
    }

    fn get_conn_v3(
        &self,
        connection: Resource<v3::Connection>,
    ) -> Result<Arc<dyn MqttClient>, v3::Error> {
        self.connections
            .get(connection.rep())
            .cloned()
            .ok_or(v3::Error::Other(
                "could not find connection for resource".into(),
            ))
    }
}

impl v3::Host for InstanceState {
    fn convert_error(&mut self, err: v3::Error) -> anyhow::Result<v3::Error> {
        Ok(err)
    }
}

impl v3::HostConnection for InstanceState {
    async fn drop(&mut self, connection: Resource<v3::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

impl v3::HostConnectionWithStore for crate::MqttFactorData {
    #[instrument(name = "spin_outbound_mqtt.open_connection", skip(accessor, password), err(level = Level::INFO), fields(otel.kind = "client"))]
    async fn open<T: Send>(
        accessor: &Accessor<T, Self>,
        address: String,
        username: String,
        password: String,
        keep_alive_interval_in_secs: u64,
    ) -> Result<Resource<v3::Connection>, v3::Error> {
        let (allowed_host_checker, create_client) = accessor.with(|mut access| {
            let host = access.get();
            host.otel.reparent_tracing_span();
            (host.allowed_hosts.clone(), host.create_client.clone())
        });

        if !allowed_host_checker
            .is_address_allowed(&address)
            .await
            .map_err(|e| v3::Error::Other(e.to_string()))?
        {
            return Err(v3::Error::ConnectionFailed(format!(
                "address {address} is not permitted"
            )));
        }

        let client = create_client
            .create(
                address,
                username,
                password,
                Duration::from_secs(keep_alive_interval_in_secs),
            )
            .unwrap();

        accessor.with(|mut access| {
            let host = access.get();
            host.connections
                .push(client)
                .map(Resource::new_own)
                .map_err(|_| v3::Error::TooManyConnections)
        })
    }

    #[instrument(name = "spin_outbound_mqtt.publish", skip(accessor, connection, payload), err(level = Level::INFO),
        fields(otel.kind = "producer", otel.name = format!("{} publish", topic), messaging.operation = "publish",
        messaging.system = "mqtt"))]
    async fn publish<T: Send>(
        accessor: &Accessor<T, Self>,
        connection: Resource<v3::Connection>,
        topic: String,
        payload: v3::Payload,
        qos: v3::Qos,
    ) -> Result<(), v3::Error> {
        let conn = accessor.with(|mut access| {
            let host = access.get();
            host.otel.reparent_tracing_span();
            host.get_conn_v3(connection)
        })?;

        conn.publish_bytes(topic, qos, payload).await?;

        Ok(())
    }
}

impl v2::Host for InstanceState {
    fn convert_error(&mut self, error: v2::Error) -> Result<v2::Error> {
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
    ) -> Result<Resource<v2::Connection>, v2::Error> {
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
        connection: Resource<v2::Connection>,
        topic: String,
        payload: Vec<u8>,
        qos: v2::Qos,
    ) -> Result<(), v2::Error> {
        self.otel.reparent_tracing_span();

        let conn = self.get_conn(connection)?;

        let qos = match qos {
            v2::Qos::AtMostOnce => v3::Qos::AtMostOnce,
            v2::Qos::AtLeastOnce => v3::Qos::AtLeastOnce,
            v2::Qos::ExactlyOnce => v3::Qos::ExactlyOnce,
        };

        conn.publish_bytes(topic, qos, payload).await?;

        Ok(())
    }

    async fn drop(&mut self, connection: Resource<v2::Connection>) -> anyhow::Result<()> {
        self.connections.remove(connection.rep());
        Ok(())
    }
}

pub fn other_error_v3(e: impl std::fmt::Display) -> v3::Error {
    v3::Error::Other(e.to_string())
}
