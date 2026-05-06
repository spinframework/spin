use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::bail;
use bytes::Bytes;
use http::{Request, Uri};
use http_body_util::{BodyExt, Empty, combinators::UnsyncBoxBody};
use spin_common::{assert_matches, assert_not_matches};
use spin_factor_outbound_http::{
    ErrorCode, HostFutureIncomingResponse, OutboundHttpFactor, SelfRequestOrigin,
    intercept::{InterceptOutcome, InterceptRequest, OutboundHttpInterceptor},
};
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_variables::VariablesFactor;
use spin_factors::{RuntimeFactors, anyhow};
use spin_factors_test::{TestEnvironment, toml};
use spin_world::async_trait;
use tracing::{
    Subscriber,
    field::{Field, Visit},
    span::{self, Record},
};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
};
use wasmtime_wasi::p2::Pollable;
use wasmtime_wasi_http::p2::types::OutgoingRequestConfig;
use wasmtime_wasi_http::p3::{RequestOptions, bindings::http::types as p3_types};

#[derive(RuntimeFactors)]
struct TestFactors {
    variables: VariablesFactor,
    networking: OutboundNetworkingFactor,
    http: OutboundHttpFactor,
}

#[tokio::test(flavor = "multi_thread")]
async fn allowed_host_is_allowed() -> anyhow::Result<()> {
    let mut state = test_instance_state("https://*", true).await?;
    let wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();

    // [100::] is the IPv6 "Discard Prefix", which should always fail
    let req = Request::get("https://[100::1]:443").body(Default::default())?;
    let mut future_resp = wasi_http.hooks.send_request(req, test_request_config())?;
    future_resp.ready().await;

    assert_discard_prefix_error(future_resp);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn self_request_smoke_test() -> anyhow::Result<()> {
    let mut state = test_instance_state("http://self", true).await?;
    // [100::] is the IPv6 "Discard Prefix", which should always fail
    let origin = SelfRequestOrigin::from_uri(&Uri::from_static("http://[100::1]"))?;
    state.http.set_self_request_origin(origin);

    let wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();
    let req = Request::get("/self-request").body(Default::default())?;
    let mut future_resp = wasi_http.hooks.send_request(req, test_request_config())?;
    future_resp.ready().await;

    assert_discard_prefix_error(future_resp);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn disallowed_host_fails() -> anyhow::Result<()> {
    let mut state = test_instance_state("https://allowed.test", true).await?;
    let wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();

    let req = Request::get("https://denied.test").body(Default::default())?;
    let mut future_resp = wasi_http.hooks.send_request(req, test_request_config())?;
    future_resp.ready().await;
    assert_matches!(
        future_resp.unwrap_ready().unwrap(),
        Err(ErrorCode::HttpRequestDenied),
    );
    Ok(())
}

#[ignore = "flaky"]
#[tokio::test(flavor = "multi_thread")]
async fn disallowed_private_ips_fails() -> anyhow::Result<()> {
    async fn run_test(allow_private_ips: bool) -> anyhow::Result<()> {
        let mut state = test_instance_state("http://*", allow_private_ips).await?;
        let wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();
        let req = Request::get("http://localhost").body(Default::default())?;
        let mut future_resp = wasi_http.hooks.send_request(req, test_request_config())?;
        future_resp.ready().await;
        match future_resp.unwrap_ready().unwrap() {
            // If we don't allow private IPs, we should not get a response
            Ok(_) if !allow_private_ips => bail!("expected Err, got Ok"),
            // Otherwise, it's fine if the request happens to succeed
            Ok(_) => {}
            // If private IPs are disallowed, we should get an error saying the destination is prohibited
            Err(err) if !allow_private_ips => {
                assert_matches!(err, ErrorCode::DestinationIpProhibited);
            }
            // Otherwise, we should get some non-DestinationIpProhibited error
            Err(err) => {
                assert_not_matches!(err, ErrorCode::DestinationIpProhibited);
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

#[tokio::test(flavor = "multi_thread")]
async fn override_connect_addr_disallowed_private_ip_fails() -> anyhow::Result<()> {
    let mut state = test_instance_state("http://*", false).await?;
    state.http.set_request_interceptor({
        struct Interceptor;
        #[async_trait]
        impl OutboundHttpInterceptor for Interceptor {
            async fn intercept(
                &self,
                mut request: InterceptRequest,
            ) -> wasmtime_wasi_http::p2::HttpResult<InterceptOutcome> {
                request.override_connect_addr("[::1]:80".parse().unwrap());
                Ok(InterceptOutcome::Continue(request))
            }
        }
        Interceptor
    })?;
    let wasi_http = OutboundHttpFactor::get_wasi_http_impl(&mut state).unwrap();
    let req = Request::get("http://1.1.1.1").body(Default::default())?;
    let mut future_resp = wasi_http.hooks.send_request(req, test_request_config())?;
    future_resp.ready().await;
    assert_matches!(
        future_resp.unwrap_ready().unwrap(),
        Err(ErrorCode::DestinationIpProhibited),
    );
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
        connect_timeout: Duration::from_millis(10),
        first_byte_timeout: Duration::from_millis(0),
        between_bytes_timeout: Duration::from_millis(0),
    }
}

fn assert_discard_prefix_error(future_resp: HostFutureIncomingResponse) {
    // Different systems handle the discard prefix differently; some will
    // immediately reject it while others will silently let it time out
    assert_matches!(
        future_resp.unwrap_ready().unwrap(),
        Err(ErrorCode::ConnectionRefused
            | ErrorCode::ConnectionTimeout
            | ErrorCode::ConnectionReadTimeout
            | ErrorCode::DnsError(_)),
    );
}

// Regression: deferred `Span::record(...)` calls (e.g. `url.full`) must
// land on the `spin_outbound_http.send_request` span created by
// `#[instrument]`. Uses the current_thread runtime so the thread-local
// subscriber covers all async work.
#[tokio::test(flavor = "current_thread")]
async fn p3_send_request_propagates_span_to_async_work() -> anyhow::Result<()> {
    let layer = CaptureLayer::default();
    let records = Arc::clone(&layer.records);
    let subscriber = tracing_subscriber::registry().with(layer);
    let _guard = tracing::subscriber::set_default(subscriber);

    let mut state = test_instance_state("https://*", true).await?;
    let p3_view = OutboundHttpFactor::get_wasi_p3_http_impl(&mut state).unwrap();
    // [100::1] is the IPv6 discard prefix — connection fails fast.
    let req = Request::get("https://[100::1]:443").body(empty_p3_body())?;
    let result_fut = p3_view
        .hooks
        .send_request(req, fast_p3_options(), p3_noop_cleanup_fut());
    let _ = Box::into_pin(result_fut).await;

    let records = records.lock().unwrap();
    assert!(
        records.iter().any(|(span, field)| {
            span == "spin_outbound_http.send_request" && field == "url.full"
        }),
        "`url.full` missing from `spin_outbound_http.send_request` span — \
         async block likely lost its `.in_current_span()` wrapper. \
         Recorded: {records:?}"
    );
    Ok(())
}

type CapturedRecords = Arc<Mutex<Vec<(String, String)>>>;

#[derive(Default)]
struct CaptureLayer {
    records: CapturedRecords,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_record(&self, id: &span::Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let span_name = ctx
            .span(id)
            .map(|s| s.name().to_string())
            .unwrap_or_default();
        let mut v = CaptureVisitor {
            span_name: &span_name,
            records: &self.records,
        };
        values.record(&mut v);
    }
}

struct CaptureVisitor<'a> {
    span_name: &'a str,
    records: &'a CapturedRecords,
}

impl Visit for CaptureVisitor<'_> {
    fn record_debug(&mut self, f: &Field, _v: &dyn std::fmt::Debug) {
        self.records
            .lock()
            .unwrap()
            .push((self.span_name.to_string(), f.name().to_string()));
    }
}

fn empty_p3_body() -> UnsyncBoxBody<Bytes, p3_types::ErrorCode> {
    Empty::<Bytes>::new()
        .map_err(|never: std::convert::Infallible| match never {})
        .boxed_unsync()
}

fn fast_p3_options() -> Option<RequestOptions> {
    Some(RequestOptions {
        connect_timeout: Some(Duration::from_millis(10)),
        first_byte_timeout: Some(Duration::from_millis(10)),
        between_bytes_timeout: Some(Duration::from_millis(10)),
    })
}

fn p3_noop_cleanup_fut()
-> Box<dyn std::future::Future<Output = Result<(), p3_types::ErrorCode>> + Send> {
    Box::new(async { Ok(()) })
}
