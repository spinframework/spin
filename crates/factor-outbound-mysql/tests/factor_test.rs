use anyhow::{bail, Result};
use spin_factor_outbound_mysql::client::Client;
use spin_factor_outbound_mysql::OutboundMysqlFactor;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_variables::VariablesFactor;
use spin_factors::{anyhow, RuntimeFactors};
use spin_factors_test::{toml, TestEnvironment};
use spin_world::async_trait;
use spin_world::v2::mysql::HostConnection;
use spin_world::v2::mysql::{self as v2};
use spin_world::v2::rdbms_types::{ParameterValue, RowSet};

#[derive(RuntimeFactors)]
struct TestFactors {
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
    mysql: OutboundMysqlFactor<MockClient>,
}

fn factors() -> TestFactors {
    TestFactors {
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
        mysql: OutboundMysqlFactor::<MockClient>::new(),
    }
}

fn test_env() -> TestEnvironment<TestFactors> {
    TestEnvironment::new(factors()).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        allowed_outbound_hosts = ["mysql://*:*"]
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
        .mysql
        .open("mysql://user:pass@mysql.test:3306/test".to_string())
        .await;
    let Err(err) = res else {
        bail!("expected Err, got Ok");
    };
    assert!(matches!(err, v2::Error::ConnectionFailed(_)));

    Ok(())
}

#[tokio::test]
async fn allowed_host_succeeds() -> anyhow::Result<()> {
    let mut state = test_env().build_instance_state().await?;

    let res = state
        .mysql
        .open("mysql://user:pass@localhost:3306/test".to_string())
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
        .mysql
        .open("mysql://user:pass@localhost:3306/test".to_string())
        .await?;

    state
        .mysql
        .execute(connection, "SELECT * FROM test".to_string(), vec![])
        .await?;

    Ok(())
}

#[tokio::test]
async fn exercise_query() -> anyhow::Result<()> {
    let mut state = test_env().build_instance_state().await?;

    let connection = state
        .mysql
        .open("mysql://user:pass@localhost:3306/test".to_string())
        .await?;

    state
        .mysql
        .query(connection, "SELECT * FROM test".to_string(), vec![])
        .await?;

    Ok(())
}

// TODO: We can expand this mock to track calls and simulate return values
pub struct MockClient {}

#[async_trait]
impl Client for MockClient {
    async fn build_client(_address: &str) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        Ok(MockClient {})
    }

    async fn execute(
        &mut self,
        _statement: String,
        _params: Vec<ParameterValue>,
    ) -> Result<(), v2::Error> {
        Ok(())
    }

    async fn query(
        &mut self,
        _statement: String,
        _params: Vec<ParameterValue>,
        _max_result_bytes: usize,
    ) -> Result<RowSet, v2::Error> {
        Ok(RowSet {
            columns: vec![],
            rows: vec![],
        })
    }
}
