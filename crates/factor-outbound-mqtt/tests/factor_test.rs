use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use spin_core::async_trait;
use spin_factor_outbound_mqtt::{ClientCreator, MqttClient, OutboundMqttFactor};
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_variables::VariablesFactor;
use spin_factors::{RuntimeFactors, anyhow};
use spin_factors_test::{TestEnvironment, toml};
use spin_world::spin::mqtt::mqtt::{Error, Qos};
use spin_world::v2::mqtt as v2_mqtt;

pub struct MockMqttClient {}

#[async_trait]
impl MqttClient for MockMqttClient {
    async fn publish_bytes(
        &self,
        _topic: String,
        _qos: Qos,
        _payload: Vec<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

impl ClientCreator for MockMqttClient {
    fn create(
        &self,
        _address: String,
        _username: String,
        _password: String,
        _keep_alive_interval: Duration,
    ) -> Result<Arc<dyn MqttClient>, Error> {
        Ok(Arc::new(MockMqttClient {}))
    }
}

#[derive(RuntimeFactors)]
struct TestFactors {
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
    mqtt: OutboundMqttFactor,
}

fn factors() -> TestFactors {
    TestFactors {
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
        mqtt: OutboundMqttFactor::new(Arc::new(MockMqttClient {})),
    }
}

fn test_env() -> TestEnvironment<TestFactors> {
    TestEnvironment::new(factors()).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        allowed_outbound_hosts = ["mqtt://*:*"]
    })
}

#[tokio::test]
async fn disallowed_host_fails() -> anyhow::Result<()> {
    use v2_mqtt::HostConnection;

    let env = TestEnvironment::new(factors()).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
    });
    let mut state = env.build_instance_state().await?;

    let res = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await;
    let Err(err) = res else {
        bail!("expected Err, got Ok");
    };
    assert!(matches!(err, v2_mqtt::Error::ConnectionFailed(_)));

    Ok(())
}

#[tokio::test]
async fn allowed_host_succeeds() -> anyhow::Result<()> {
    use v2_mqtt::HostConnection;

    let mut state = test_env().build_instance_state().await?;

    let res = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await;
    let Ok(_) = res else {
        bail!("expected Ok, got Err");
    };

    Ok(())
}

#[tokio::test]
async fn exercise_publish() -> anyhow::Result<()> {
    use v2_mqtt::HostConnection;

    let mut state = test_env().build_instance_state().await?;

    let res = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await?;

    state
        .mqtt
        .publish(
            res,
            "message".to_string(),
            b"test message".to_vec(),
            v2_mqtt::Qos::ExactlyOnce,
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn oversized_payload_rejected() -> anyhow::Result<()> {
    use v2_mqtt::HostConnection;

    const LIMIT: usize = 10;

    let env = test_env().runtime_config(TestFactorsRuntimeConfig {
        mqtt: Some(spin_factor_outbound_mqtt::runtime_config::RuntimeConfig {
            max_payload_size_bytes: Some(LIMIT),
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let mut state = env.build_instance_state().await?;

    let conn = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await?;

    let oversized = vec![0u8; LIMIT + 1];
    let err = state
        .mqtt
        .publish(
            conn,
            "topic".to_string(),
            oversized,
            v2_mqtt::Qos::AtMostOnce,
        )
        .await;
    assert!(
        matches!(err, Err(v2_mqtt::Error::Other(_))),
        "expected Other error for oversized payload, got {err:?}"
    );

    Ok(())
}

#[tokio::test]
async fn payload_at_limit_succeeds() -> anyhow::Result<()> {
    use v2_mqtt::HostConnection;

    const LIMIT: usize = 10;

    let env = test_env().runtime_config(TestFactorsRuntimeConfig {
        mqtt: Some(spin_factor_outbound_mqtt::runtime_config::RuntimeConfig {
            max_payload_size_bytes: Some(LIMIT),
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let mut state = env.build_instance_state().await?;

    let conn = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await?;

    let exactly_limit = vec![0u8; LIMIT];
    state
        .mqtt
        .publish(
            conn,
            "topic".to_string(),
            exactly_limit,
            v2_mqtt::Qos::AtMostOnce,
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn connection_limit_blocks_when_exhausted() -> anyhow::Result<()> {
    use v2_mqtt::HostConnection;

    let env = TestEnvironment::new(factors())
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["mqtt://*:*"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            mqtt: Some(spin_factor_outbound_mqtt::runtime_config::RuntimeConfig {
                max_connections: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let mut state = env.build_instance_state().await?;

    // Open first connection - should succeed immediately.
    let conn1 = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await?;

    // Second open should block (wait for a permit) since the limit is 1.
    let timed_out = tokio::time::timeout(
        Duration::from_millis(10),
        state.mqtt.open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        ),
    )
    .await
    .is_err();
    assert!(timed_out, "expected second open to block when limit is 1");

    // Releasing the first connection returns its permit to the semaphore.
    state.mqtt.drop(conn1).await?;

    // Now a new connection should succeed.
    let conn2 = state
        .mqtt
        .open(
            "mqtt://mqtt.test:1883".to_string(),
            "username".to_string(),
            "password".to_string(),
            1,
        )
        .await?;
    state.mqtt.drop(conn2).await?;

    Ok(())
}
