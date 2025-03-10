use anyhow::{bail, Result};
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_outbound_pg::client::Client;
use spin_factor_outbound_pg::client::ClientFactory;
use spin_factor_outbound_pg::OutboundPgFactor;
use spin_factor_variables::VariablesFactor;
use spin_factors::{anyhow, RuntimeFactors};
use spin_factors_test::{toml, TestEnvironment};
use spin_world::async_trait;
use spin_world::spin::postgres::postgres::Error as PgError;
use spin_world::spin::postgres::postgres::HostConnection;
use spin_world::spin::postgres::postgres::{self as v2};
use spin_world::spin::postgres::postgres::{ParameterValue, RowSet};

#[derive(RuntimeFactors)]
struct TestFactors {
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
    pg: OutboundPgFactor<MockClientFactory>,
}

fn factors() -> TestFactors {
    TestFactors {
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
        pg: OutboundPgFactor::<MockClientFactory>::new(),
    }
}

fn test_env() -> TestEnvironment<TestFactors> {
    TestEnvironment::new(factors()).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        allowed_outbound_hosts = ["postgres://*:*"]
    })
}

#[tokio::test]
async fn disallowed_host_fails() -> anyhow::Result<()> {
    let env = TestEnvironment::new(factors()).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
    });
    let mut state = env.build_instance_state().await?;

    let res = state
        .pg
        .open("postgres://postgres.test:5432/test".to_string())
        .await;
    let Err(err) = res else {
        bail!("expected Err, got Ok");
    };
    assert!(matches!(err, PgError::ConnectionFailed(_)));

    Ok(())
}

#[tokio::test]
async fn allowed_host_succeeds() -> anyhow::Result<()> {
    let mut state = test_env().build_instance_state().await?;

    let res = state
        .pg
        .open("postgres://localhost:5432/test".to_string())
        .await;
    let Ok(_) = res else {
        bail!("expected Ok, got Err");
    };

    Ok(())
}

#[tokio::test]
async fn exercise_execute() -> anyhow::Result<()> {
    let mut state = test_env().build_instance_state().await?;

    let connection = state
        .pg
        .open("postgres://localhost:5432/test".to_string())
        .await?;

    state
        .pg
        .execute(connection, "SELECT * FROM test".to_string(), vec![])
        .await?;

    Ok(())
}

#[tokio::test]
async fn exercise_query() -> anyhow::Result<()> {
    let mut state = test_env().build_instance_state().await?;

    let connection = state
        .pg
        .open("postgres://localhost:5432/test".to_string())
        .await?;

    state
        .pg
        .query(connection, "SELECT * FROM test".to_string(), vec![])
        .await?;

    Ok(())
}

// TODO: We can expand this mock to track calls and simulate return values
pub struct MockClientFactory {}
pub struct MockClient {}

#[async_trait]
impl ClientFactory for MockClientFactory {
    type Client = MockClient;
    fn new() -> Self {
        Self {}
    }
    async fn build_client(&mut self, _address: &str) -> Result<Self::Client> {
        Ok(MockClient {})
    }
}

#[async_trait]
impl Client for MockClient {
    async fn execute(
        &self,
        _statement: String,
        _params: Vec<ParameterValue>,
    ) -> Result<u64, v2::Error> {
        Ok(0)
    }

    async fn query(
        &self,
        _statement: String,
        _params: Vec<ParameterValue>,
    ) -> Result<RowSet, v2::Error> {
        Ok(RowSet {
            columns: vec![],
            rows: vec![],
        })
    }
}
