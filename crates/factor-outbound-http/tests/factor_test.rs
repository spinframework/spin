use std::time::Duration;

use anyhow::bail;
use http::{Request, Uri};
use spin_factor_outbound_http::{OutboundHttpFactor, SelfRequestOrigin};
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_variables::VariablesFactor;
use spin_factors::{anyhow, RuntimeFactors};
use spin_factors_test::{toml, TestEnvironment};
use wasmtime_wasi::p2::Pollable;
use wasmtime_wasi_http::{
    bindings::http::types::ErrorCode, types::OutgoingRequestConfig, WasiHttpView,
};

#[derive(RuntimeFactors)]
struct TestFactors {
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
    http: OutboundHttpFactor,
}

#[tokio::test]
async fn allowed_host_is_allowed() -> anyhow::Result<()> {
    let mut state = test_instance_state("https://*", true).await?;
    let mut wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();

    // [100::] is the IPv6 "Discard Prefix", which should always fail
    let req = Request::get("https://[100::1]:443").body(Default::default())?;
    let mut future_resp = wasi_http.send_request(req, test_request_config())?;
    future_resp.ready().await;

    // Different systems handle the discard prefix differently; some will
    // immediately reject it while others will silently let it time out
    match future_resp.unwrap_ready().unwrap() {
        Err(ErrorCode::ConnectionRefused | ErrorCode::ConnectionTimeout) => (),
        other => bail!("expected Err(ConnectionRefused | ConnectionTimeout), got {other:?}"),
    };
    Ok(())
}

#[tokio::test]
async fn self_request_smoke_test() -> anyhow::Result<()> {
    let mut state = test_instance_state("http://self", true).await?;
    // [100::] is the IPv6 "Discard Prefix", which should always fail
    let origin = SelfRequestOrigin::from_uri(&Uri::from_static("http://[100::1]"))?;
    state.http.set_self_request_origin(origin);

    let mut wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();
    let req = Request::get("/self-request").body(Default::default())?;
    let mut future_resp = wasi_http.send_request(req, test_request_config())?;
    future_resp.ready().await;

    // Different systems handle the discard prefix differently; some will
    // immediately reject it while others will silently let it time out
    match future_resp.unwrap_ready().unwrap() {
        Err(ErrorCode::ConnectionRefused | ErrorCode::ConnectionTimeout) => (),
        other => bail!("expected Err(ConnectionRefused | ConnectionTimeout), got {other:?}"),
    };
    Ok(())
}

#[tokio::test]
async fn disallowed_host_fails() -> anyhow::Result<()> {
    let mut state = test_instance_state("https://allowed.test", true).await?;
    let mut wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();

    let req = Request::get("https://denied.test").body(Default::default())?;
    let mut future_resp = wasi_http.send_request(req, test_request_config())?;
    future_resp.ready().await;
    match future_resp.unwrap_ready().unwrap() {
        Ok(_) => bail!("expected Err, got Ok"),
        Err(err) => assert!(matches!(err, ErrorCode::HttpRequestDenied)),
    };
    Ok(())
}

#[tokio::test]
async fn disallowed_private_ips_fails() -> anyhow::Result<()> {
    async fn run_test(allow_private_ips: bool) -> anyhow::Result<()> {
        let mut state = test_instance_state("http://*", allow_private_ips).await?;
        let mut wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();
        let req = Request::get("http://localhost").body(Default::default())?;
        let mut future_resp = wasi_http.send_request(req, test_request_config())?;
        future_resp.ready().await;
        match future_resp.unwrap_ready().unwrap() {
            // If we don't allow private IPs, we should not get a response
            Ok(_) if !allow_private_ips => bail!("expected Err, got Ok"),
            // Otherwise, it's fine if the request happens to succeed
            Ok(_) => {}
            // If private IPs are disallowed, we should get an error saying the destination is prohibited
            Err(err) if !allow_private_ips => {
                assert!(matches!(err, ErrorCode::DestinationIpProhibited))
            }
            // Otherwise, we should get some non-DestinationIpProhibited error
            Err(err) => {
                assert!(!matches!(err, ErrorCode::DestinationIpProhibited))
            }
        };
        Ok(())
    }

    // Test with private IPs allowed
    run_test(true).await?;
    // Test with private IPs disallowed
    run_test(false).await?;

    Ok(())
}

async fn test_instance_state(
    allowed_outbound_hosts: &str,
    allow_private_ips: bool,
) -> anyhow::Result<TestFactorsInstanceState> {
    let factors = TestFactors {
        variables: VariablesFactor::default(),
        networking: OutboundNetworkingFactor::new(),
        http: OutboundHttpFactor::default(),
    };
    let env = TestEnvironment::new(factors)
        .extend_manifest(toml! {
            [component.test-component]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = [allowed_outbound_hosts]
        })
        .runtime_config(TestFactorsRuntimeConfig {
            networking: Some(
                spin_factor_outbound_networking::runtime_config::RuntimeConfig {
                    block_private_networks: !allow_private_ips,
                    ..Default::default()
                },
            ),
            ..Default::default()
        })?;
    env.build_instance_state().await
}

fn test_request_config() -> OutgoingRequestConfig {
    OutgoingRequestConfig {
        use_tls: false,
        connect_timeout: Duration::from_millis(1),
        first_byte_timeout: Duration::from_millis(1),
        between_bytes_timeout: Duration::from_millis(1),
    }
}
