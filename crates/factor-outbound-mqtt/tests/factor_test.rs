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
use spin_world::v2::mqtt as v2;

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
    use v2::HostConnection;

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
    assert!(matches!(err, v2::Error::ConnectionFailed(_)));

    Ok(())
}

#[tokio::test]
async fn allowed_host_succeeds() -> anyhow::Result<()> {
    use v2::HostConnection;

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
    use v2::HostConnection;

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
            v2::Qos::ExactlyOnce,
        )
        .await?;

    Ok(())
}

#[tokio::test]
async fn oversized_payload_rejected() -> anyhow::Result<()> {
    use v2::HostConnection;

    const LIMIT: usize = 10;

    let env = test_env().runtime_config(TestFactorsRuntimeConfig {
        mqtt: Some(spin_factor_outbound_mqtt::runtime_config::RuntimeConfig {
            max_payload_size_bytes: Some(LIMIT),
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
        .publish(conn, "topic".to_string(), oversized, v2::Qos::AtMostOnce)
        .await;
    assert!(
        matches!(err, Err(v2::Error::Other(_))),
        "expected Other error for oversized payload, got {err:?}"
    );

    Ok(())
}

#[tokio::test]
async fn payload_at_limit_succeeds() -> anyhow::Result<()> {
    use v2::HostConnection;

    const LIMIT: usize = 10;

    let env = test_env().runtime_config(TestFactorsRuntimeConfig {
        mqtt: Some(spin_factor_outbound_mqtt::runtime_config::RuntimeConfig {
            max_payload_size_bytes: Some(LIMIT),
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
            v2::Qos::AtMostOnce,
        )
        .await?;

    Ok(())
}
