use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_outbound_networking::runtime_config::RuntimeConfig;
use spin_factor_outbound_networking::runtime_config::spin::SpinRuntimeConfig;
use spin_factor_variables::VariablesFactor;
use spin_factor_wasi::{DummyFilesMounter, WasiFactor};
use spin_factors::anyhow::Context as _;
use spin_factors::{App, RuntimeFactors, anyhow};
use spin_factors_test::{TestEnvironment, toml};
use wasmtime_wasi::p2::bindings::sockets::instance_network::Host;
use wasmtime_wasi::p2::bindings::sockets::network::{ErrorCode, IpAddressFamily};
use wasmtime_wasi::p2::bindings::sockets::tcp as p2_tcp;
use wasmtime_wasi::p2::bindings::sockets::tcp_create_socket as p2_tcp_create;
use wasmtime_wasi::p2::bindings::sockets::udp_create_socket as p2_udp_create;
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

#[tokio::test]
async fn socket_quota_blocks_excess_connections() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(RuntimeConfig {
                max_sockets_per_app: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();
    let addr: std::net::SocketAddr = "123.0.2.1:12345".parse().unwrap();

    // First two connections should be accepted (non-blocking connect initiated)
    let net1 = sockets.instance_network()?;
    let sock1 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock1, net1, addr.into()).await?;

    let net2 = sockets.instance_network()?;
    let sock2 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock2, net2, addr.into()).await?;

    // Third should fail — quota exhausted
    let net3 = sockets.instance_network()?;
    let sock3 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    let err = p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock3, net3, addr.into())
        .await
        .unwrap_err();
    assert_eq!(err.downcast_ref(), Some(&ErrorCode::ConnectionRefused));
    Ok(())
}

#[tokio::test]
async fn socket_quota_releases_on_instance_drop() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(RuntimeConfig {
                max_sockets_per_app: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let locked_app = env.build_locked_app().await?;
    let TestEnvironment {
        factors,
        runtime_config,
        ..
    } = env;
    let app = App::new("test-app", locked_app);
    let configured_app = factors.configure_app(app, runtime_config)?;
    let component_id = configured_app
        .app()
        .components()
        .last()
        .context("no components")?
        .id()
        .to_string();

    let addr: std::net::SocketAddr = "123.0.2.1:12345".parse().unwrap();

    // First instance: fill the quota (1 socket)
    {
        let builders = factors.prepare(&configured_app, &component_id)?;
        let mut state = factors.build_instance_state(builders)?;
        let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();
        let net = sockets.instance_network()?;
        let sock = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
        p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock, net, addr.into()).await?;
        // sockets state dropped here releasing the permit back to the semaphore
    }

    // Second instance: quota should be fully available again
    let builders = factors.prepare(&configured_app, &component_id)?;
    let mut state = factors.build_instance_state(builders)?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();
    let net = sockets.instance_network()?;
    let sock = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock, net, addr.into()).await?;
    Ok(())
}

#[tokio::test]
async fn no_socket_quota_allows_unlimited() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        allowed_outbound_hosts = ["*://123.0.2.1:12345"]
    });

    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();
    let addr: std::net::SocketAddr = "123.0.2.1:12345".parse().unwrap();

    for _ in 0..10 {
        let net = sockets.instance_network()?;
        let sock = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
        p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock, net, addr.into()).await?;
    }
    Ok(())
}

#[tokio::test]
async fn socket_quota_still_enforces_allowed_hosts() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(RuntimeConfig {
                max_sockets_per_app: Some(10),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();

    // Allowed host succeeds
    let net = sockets.instance_network()?;
    let sock = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    let allowed_addr: std::net::SocketAddr = "123.0.2.1:12345".parse().unwrap();
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock, net, allowed_addr.into()).await?;

    // Disallowed host is rejected even with quota available
    let net = sockets.instance_network()?;
    let sock = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    let disallowed_addr: std::net::SocketAddr = "1.2.3.4:80".parse().unwrap();
    assert!(
        p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock, net, disallowed_addr.into())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn socket_quota_releases_on_socket_drop() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(RuntimeConfig {
                max_sockets_per_app: Some(1),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();
    let addr: std::net::SocketAddr = "123.0.2.1:12345".parse().unwrap();

    // Acquire the only permit via start_connect. Save the rep so we can reconstruct
    // a handle afterwards — start_connect consumes the Resource but leaves the socket
    // alive in the ResourceTable.
    let net1 = sockets.instance_network()?;
    let sock1 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    let sock1_rep = sock1.rep();
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock1, net1, addr.into()).await?;

    // A second start_connect should fail while the permit is held.
    let net2 = sockets.instance_network()?;
    let sock2 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    let err = p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock2, net2, addr.into())
        .await
        .unwrap_err();
    assert_eq!(err.downcast_ref(), Some(&ErrorCode::ConnectionRefused));

    // Explicitly drop sock1 before finish_connect — this should release the permit.
    let sock1_handle =
        wasmtime::component::Resource::<wasmtime_wasi::sockets::TcpSocket>::new_own(sock1_rep);
    p2_tcp::HostTcpSocket::drop(&mut sockets, sock1_handle)?;

    // After the drop the quota is free again, so a new start_connect must succeed.
    let net3 = sockets.instance_network()?;
    let sock3 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, sock3, net3, addr.into()).await?;

    Ok(())
}

#[tokio::test]
async fn socket_quota_blocks_excess_udp_sockets() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(RuntimeConfig {
                max_sockets_per_app: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();

    // First two UDP socket creations should succeed.
    p2_udp_create::Host::create_udp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    p2_udp_create::Host::create_udp_socket(&mut sockets, IpAddressFamily::Ipv4)?;

    // Third should fail — quota exhausted.
    let err =
        p2_udp_create::Host::create_udp_socket(&mut sockets, IpAddressFamily::Ipv4).unwrap_err();
    assert_eq!(err.downcast_ref(), Some(&ErrorCode::ConnectionRefused));
    Ok(())
}

#[tokio::test]
async fn socket_quota_shared_between_tcp_and_udp() -> anyhow::Result<()> {
    let factors = TestFactors {
        wasi: WasiFactor::new(DummyFilesMounter),
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["*://123.0.2.1:12345"]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(RuntimeConfig {
                max_sockets_per_app: Some(2),
                ..Default::default()
            }),
            ..Default::default()
        })?;

    let mut state = env.build_instance_state().await?;
    let mut sockets = WasiFactor::get_sockets_impl(&mut state).unwrap();
    let addr: std::net::SocketAddr = "123.0.2.1:12345".parse().unwrap();

    // Consume one permit with a TCP connection.
    let net = sockets.instance_network()?;
    let tcp_sock = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    p2_tcp::HostTcpSocket::start_connect(&mut sockets, tcp_sock, net, addr.into()).await?;

    // Consume the second permit with a UDP socket — quota now full.
    p2_udp_create::Host::create_udp_socket(&mut sockets, IpAddressFamily::Ipv4)?;

    // Any further allocation must fail — shared quota exhausted.
    // UDP:
    let err =
        p2_udp_create::Host::create_udp_socket(&mut sockets, IpAddressFamily::Ipv4).unwrap_err();
    assert_eq!(err.downcast_ref(), Some(&ErrorCode::ConnectionRefused));
    // TCP:
    let net = sockets.instance_network()?;
    let tcp_sock2 = p2_tcp_create::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4)?;
    let err = p2_tcp::HostTcpSocket::start_connect(&mut sockets, tcp_sock2, net, addr.into())
        .await
        .unwrap_err();
    assert_eq!(err.downcast_ref(), Some(&ErrorCode::ConnectionRefused));
    Ok(())
}
