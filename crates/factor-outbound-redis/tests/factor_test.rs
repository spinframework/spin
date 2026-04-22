use anyhow::bail;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_outbound_redis::OutboundRedisFactor;
use spin_factor_variables::VariablesFactor;
use spin_factors::{RuntimeFactors, anyhow};
use spin_factors_test::{TestEnvironment, toml};
use spin_world::v2::redis::{Error, HostConnection};

#[derive(RuntimeFactors)]
struct TestFactors {
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
    redis: OutboundRedisFactor,
}

#[tokio::test]
async fn no_outbound_hosts_fails() -> anyhow::Result<()> {
    let factors = TestFactors {
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
        redis: OutboundRedisFactor::new(),
    };
    let env = TestEnvironment::new(factors).extend_manifest(toml! {
        spin_manifest_version = 2
        application.name = "test-app"
        [[trigger.test]]

        [component.test-component]
        source = "does-not-exist.wasm"
    });
    let mut state = env.build_instance_state().await?;
    let connection = state
        .redis
        .open("redis://redis.test:8080".to_string())
        .await;

    let Err(err) = connection else {
        bail!("expected Error, got Ok");
    };

    assert!(matches!(err, Error::InvalidAddress));
    Ok(())
}
