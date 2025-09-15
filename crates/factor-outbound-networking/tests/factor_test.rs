use spin_factor_outbound_networking::runtime_config::spin::SpinRuntimeConfig;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_variables::VariablesFactor;
use spin_factor_wasi::{DummyFilesMounter, WasiFactor};
use spin_factors::{anyhow, RuntimeFactors};
use spin_factors_test::{toml, TestEnvironment};
use wasmtime_wasi::p2::bindings::sockets::instance_network::Host;
use wasmtime_wasi::sockets::SocketAddrUse;

#[derive(RuntimeFactors)]
struct TestFactors {
    wasi: WasiFactor,
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
}

#[tokio::test]
async fn configures_wasi_socket_addr_check() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345", "*://123.123.0.1:443", "*://127.0.0.1:3000"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: SpinRuntimeConfig::new("").config_from_table(&toml! {
                [outbound_networking]
                block_networks = ["123.123.123.123/16", "private"]
            })?,
            ..Default::default()
        })?;
    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();

    let network_resource = sockets.instance_network()?;
    let network = sockets.table.get(&network_resource)?;

    network
        .check_socket_addr(
            "123.0.2.1:12345".parse().unwrap(),
            SocketAddrUse::TcpConnect,
        )
        .await?;
    for not_allowed in [
        // Blocked by allowed_outbound_hosts
        "123.0.2.1:25",
        "123.0.2.2:12345",
        // Blocked by block_networks
        "123.123.0.1:443",
        "127.0.0.1:3000",
    ] {
        assert_eq!(
            network
                .check_socket_addr(not_allowed.parse().unwrap(), SocketAddrUse::TcpConnect)
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
    Ok(())
}

#[tokio::test]
async fn wasi_factor_is_optional() -> anyhow::Result<()> {
    #[derive(RuntimeFactors)]
    struct WithoutWasi {
        variables: VariablesFactor,
        networking: OutboundNetworkingFactor,
    }
    TestEnvironment::new(WithoutWasi {
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    })
    .build_instance_state()
    .await?;
    Ok(())
}
