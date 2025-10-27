//! Implementation for the Spin HTTP engine.

mod headers;
mod instrument;
mod outbound_http;
mod server;
mod spin;
mod tls;
mod wagi;
mod wasi;
mod wasip3;

use std::{
    error::Error,
    fmt::Display,
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, Context};
use clap::Args;
use rand::{
    distr::uniform::{SampleRange, SampleUniform},
    RngCore,
};
use serde::Deserialize;
use spin_app::App;
use spin_factors::RuntimeFactors;
use spin_trigger::Trigger;
use wasmtime_wasi_http::bindings::http::types::ErrorCode;

pub use server::HttpServer;

pub use tls::TlsConfig;

pub(crate) use wasmtime_wasi_http::body::HyperIncomingBody as Body;

const DEFAULT_WASIP3_MAX_INSTANCE_REUSE_COUNT: usize = 128;
const DEFAULT_WASIP3_MAX_INSTANCE_CONCURRENT_REUSE_COUNT: usize = 16;
const DEFAULT_REQUEST_TIMEOUT: Option<Range<Duration>> = None;
const DEFAULT_IDLE_INSTANCE_TIMEOUT: Range<Duration> = Range::Value(Duration::from_secs(1));

/// A [`spin_trigger::TriggerApp`] for the HTTP trigger.
pub(crate) type TriggerApp<F> = spin_trigger::TriggerApp<HttpTrigger, F>;

/// A [`spin_trigger::TriggerInstanceBuilder`] for the HTTP trigger.
pub(crate) type TriggerInstanceBuilder<'a, F> =
    spin_trigger::TriggerInstanceBuilder<'a, HttpTrigger, F>;

#[derive(Args)]
pub struct CliArgs {
    /// IP address and port to listen on
    #[clap(long = "listen", env = "SPIN_HTTP_LISTEN_ADDR", default_value = "127.0.0.1:3000", value_parser = parse_listen_addr)]
    pub address: SocketAddr,

    /// The path to the certificate to use for https, if this is not set, normal http will be used. The cert should be in PEM format
    #[clap(long, env = "SPIN_TLS_CERT", requires = "tls-key")]
    pub tls_cert: Option<PathBuf>,

    /// The path to the certificate key to use for https, if this is not set, normal http will be used. The key should be in PKCS#8 format
    #[clap(long, env = "SPIN_TLS_KEY", requires = "tls-cert")]
    pub tls_key: Option<PathBuf>,

    /// Sets the maximum buffer size (in bytes) for the HTTP connection. The minimum value allowed is 8192.
    #[clap(long, env = "SPIN_HTTP1_MAX_BUF_SIZE")]
    pub http1_max_buf_size: Option<usize>,

    #[clap(long = "find-free-port")]
    pub find_free_port: bool,

    /// Maximum number of requests to send to a single component instance before
    /// dropping it.
    ///
    /// This defaults to 1 for WASIp2 components and 128 for WASIp3 components.
    /// As of this writing, setting it to more than 1 will have no effect for
    /// WASIp2 components, but that may change in the future.
    ///
    /// This may be specified either as an integer value or as a range,
    /// e.g. 1..8.  If it's a range, a number will be selected from that range
    /// at random for each new instance.
    #[clap(long, value_parser = parse_usize_range)]
    max_instance_reuse_count: Option<Range<usize>>,

    /// Maximum number of concurrent requests to send to a single component
    /// instance.
    ///
    /// This defaults to 1 for WASIp2 components and 16 for WASIp3 components.
    /// Note that setting it to more than 1 will have no effect for WASIp2
    /// components since they cannot be called concurrently.
    ///
    /// This may be specified either as an integer value or as a range,
    /// e.g. 1..8.  If it's a range, a number will be selected from that range
    /// at random for each new instance.
    #[clap(long, value_parser = parse_usize_range)]
    max_instance_concurrent_reuse_count: Option<Range<usize>>,

    /// Request timeout to enforce.
    ///
    /// As of this writing, this only affects WASIp3 components.
    ///
    /// A number with no suffix or with an `s` suffix is interpreted as seconds;
    /// other accepted suffixes include `ms` (milliseconds), `us` or `μs`
    /// (microseconds), and `ns` (nanoseconds).
    ///
    /// This may be specified either as a single time value or as a range,
    /// e.g. 1..8s.  If it's a range, a value will be selected from that range
    /// at random for each new instance.
    #[clap(long, value_parser = parse_duration_range)]
    request_timeout: Option<Range<Duration>>,

    /// Time to hold an idle component instance for possible reuse before
    /// dropping it.
    ///
    /// A number with no suffix or with an `s` suffix is interpreted as seconds;
    /// other accepted suffixes include `ms` (milliseconds), `us` or `μs`
    /// (microseconds), and `ns` (nanoseconds).
    ///
    /// This may be specified either as a single time value or as a range,
    /// e.g. 1..8s.  If it's a range, a value will be selected from that range
    /// at random for each new instance.
    #[clap(long, default_value = "1s", value_parser = parse_duration_range)]
    idle_instance_timeout: Range<Duration>,
}

impl CliArgs {
    fn into_tls_config(self) -> Option<TlsConfig> {
        match (self.tls_cert, self.tls_key) {
            (Some(cert_path), Some(key_path)) => Some(TlsConfig {
                cert_path,
                key_path,
            }),
            (None, None) => None,
            _ => unreachable!(),
        }
    }
}

#[derive(Copy, Clone)]
enum Range<T> {
    Value(T),
    Bounds(T, T),
}

impl<T> Range<T> {
    fn map<V>(self, fun: impl Fn(T) -> V) -> Range<V> {
        match self {
            Self::Value(v) => Range::Value(fun(v)),
            Self::Bounds(a, b) => Range::Bounds(fun(a), fun(b)),
        }
    }
}

impl<T: SampleUniform + PartialOrd> SampleRange<T> for Range<T> {
    fn sample_single<R: RngCore + ?Sized>(
        self,
        rng: &mut R,
    ) -> Result<T, rand::distr::uniform::Error> {
        match self {
            Self::Value(v) => Ok(v),
            Self::Bounds(a, b) => (a..b).sample_single(rng),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Value(_) => false,
            Self::Bounds(a, b) => (a..b).is_empty(),
        }
    }
}

fn parse_range<T: FromStr>(s: &str) -> Result<Range<T>, String>
where
    T::Err: Display,
{
    let error = |e| format!("expected integer or range; got {s:?}; {e}");
    if let Some((start, end)) = s.split_once("..") {
        Ok(Range::Bounds(
            start.parse().map_err(error)?,
            end.parse().map_err(error)?,
        ))
    } else {
        Ok(Range::Value(s.parse().map_err(error)?))
    }
}

fn parse_usize_range(s: &str) -> Result<Range<usize>, String> {
    parse_range(s)
}

struct ParsedDuration(Duration);

impl FromStr for ParsedDuration {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = |e| {
            format!("expected integer suffixed by `s`, `ms`, `us`, `μs`, or `ns`; got {s:?}; {e}")
        };
        Ok(Self(match s.parse() {
            Ok(val) => Duration::from_secs(val),
            Err(err) => {
                if let Some(num) = s.strip_suffix("s") {
                    Duration::from_secs(num.parse().map_err(error)?)
                } else if let Some(num) = s.strip_suffix("ms") {
                    Duration::from_millis(num.parse().map_err(error)?)
                } else if let Some(num) = s.strip_suffix("us").or(s.strip_suffix("μs")) {
                    Duration::from_micros(num.parse().map_err(error)?)
                } else if let Some(num) = s.strip_suffix("ns") {
                    Duration::from_nanos(num.parse().map_err(error)?)
                } else {
                    return Err(error(err));
                }
            }
        }))
    }
}

fn parse_duration_range(s: &str) -> Result<Range<Duration>, String> {
    parse_range::<ParsedDuration>(s).map(|v| v.map(|v| v.0))
}

#[derive(Clone, Copy)]
pub struct InstanceReuseConfig {
    max_instance_reuse_count: Range<usize>,
    max_instance_concurrent_reuse_count: Range<usize>,
    request_timeout: Option<Range<Duration>>,
    idle_instance_timeout: Range<Duration>,
}

impl Default for InstanceReuseConfig {
    fn default() -> Self {
        Self {
            max_instance_reuse_count: Range::Value(DEFAULT_WASIP3_MAX_INSTANCE_REUSE_COUNT),
            max_instance_concurrent_reuse_count: Range::Value(
                DEFAULT_WASIP3_MAX_INSTANCE_CONCURRENT_REUSE_COUNT,
            ),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            idle_instance_timeout: DEFAULT_IDLE_INSTANCE_TIMEOUT,
        }
    }
}

/// The Spin HTTP trigger.
pub struct HttpTrigger {
    /// The address the server should listen on.
    ///
    /// Note that this might not be the actual socket address that ends up being bound to.
    /// If the port is set to 0, the actual address will be determined by the OS.
    listen_addr: SocketAddr,
    tls_config: Option<TlsConfig>,
    find_free_port: bool,
    http1_max_buf_size: Option<usize>,
    reuse_config: InstanceReuseConfig,
}

impl<F: RuntimeFactors> Trigger<F> for HttpTrigger {
    const TYPE: &'static str = "http";

    type CliArgs = CliArgs;
    type InstanceState = ();

    fn new(cli_args: Self::CliArgs, app: &spin_app::App) -> anyhow::Result<Self> {
        let find_free_port = cli_args.find_free_port;
        let http1_max_buf_size = cli_args.http1_max_buf_size;
        let reuse_config = InstanceReuseConfig {
            max_instance_reuse_count: cli_args
                .max_instance_reuse_count
                .unwrap_or(Range::Value(DEFAULT_WASIP3_MAX_INSTANCE_REUSE_COUNT)),
            max_instance_concurrent_reuse_count: cli_args
                .max_instance_concurrent_reuse_count
                .unwrap_or(Range::Value(
                    DEFAULT_WASIP3_MAX_INSTANCE_CONCURRENT_REUSE_COUNT,
                )),
            request_timeout: cli_args.request_timeout,
            idle_instance_timeout: cli_args.idle_instance_timeout,
        };

        Self::new(
            app,
            cli_args.address,
            cli_args.into_tls_config(),
            find_free_port,
            http1_max_buf_size,
            reuse_config,
        )
    }

    async fn run(self, trigger_app: TriggerApp<F>) -> anyhow::Result<()> {
        let server = self.into_server(trigger_app)?;

        server.serve().await?;

        Ok(())
    }

    fn supported_host_requirements() -> Vec<&'static str> {
        vec![spin_app::locked::SERVICE_CHAINING_KEY]
    }
}

impl HttpTrigger {
    /// Create a new `HttpTrigger`.
    pub fn new(
        app: &spin_app::App,
        listen_addr: SocketAddr,
        tls_config: Option<TlsConfig>,
        find_free_port: bool,
        http1_max_buf_size: Option<usize>,
        reuse_config: InstanceReuseConfig,
    ) -> anyhow::Result<Self> {
        Self::validate_app(app)?;

        Ok(Self {
            listen_addr,
            tls_config,
            find_free_port,
            http1_max_buf_size,
            reuse_config,
        })
    }

    /// Turn this [`HttpTrigger`] into an [`HttpServer`].
    pub fn into_server<F: RuntimeFactors>(
        self,
        trigger_app: TriggerApp<F>,
    ) -> anyhow::Result<Arc<HttpServer<F>>> {
        let Self {
            listen_addr,
            tls_config,
            find_free_port,
            http1_max_buf_size,
            reuse_config,
        } = self;
        let server = Arc::new(HttpServer::new(
            listen_addr,
            tls_config,
            find_free_port,
            trigger_app,
            http1_max_buf_size,
            reuse_config,
        )?);
        Ok(server)
    }

    fn validate_app(app: &App) -> anyhow::Result<()> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TriggerMetadata {
            base: Option<String>,
        }
        if let Some(TriggerMetadata { base: Some(base) }) = app.get_trigger_metadata("http")? {
            if base == "/" {
                tracing::warn!(
                    "This application has the deprecated trigger 'base' set to the default value '/'. This may be an error in the future!"
                );
            } else {
                bail!(
                    "This application is using the deprecated trigger 'base' field. The base must be prepended to each [[trigger.http]]'s 'route'."
                )
            }
        }
        Ok(())
    }
}

fn parse_listen_addr(addr: &str) -> anyhow::Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = addr.to_socket_addrs()?.collect();
    // Prefer 127.0.0.1 over e.g. [::1] because CHANGE IS HARD
    if let Some(addr) = addrs
        .iter()
        .find(|addr| addr.is_ipv4() && addr.ip() == Ipv4Addr::LOCALHOST)
    {
        return Ok(*addr);
    }
    // Otherwise, take the first addr (OS preference)
    addrs.into_iter().next().context("couldn't resolve address")
}

#[derive(Debug, PartialEq)]
enum NotFoundRouteKind {
    Normal(String),
    WellKnown,
}

/// Translate a [`hyper::Error`] to a wasi-http `ErrorCode` in the context of a request.
pub fn hyper_request_error(err: hyper::Error) -> ErrorCode {
    // If there's a source, we might be able to extract a wasi-http error from it.
    if let Some(cause) = err.source() {
        if let Some(err) = cause.downcast_ref::<ErrorCode>() {
            return err.clone();
        }
    }

    tracing::warn!("hyper request error: {err:?}");

    ErrorCode::HttpProtocolError
}

pub fn dns_error(rcode: String, info_code: u16) -> ErrorCode {
    ErrorCode::DnsError(wasmtime_wasi_http::bindings::http::types::DnsErrorPayload {
        rcode: Some(rcode),
        info_code: Some(info_code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_listen_addr_prefers_ipv4() {
        let addr = parse_listen_addr("localhost:12345").unwrap();
        assert_eq!(addr.ip(), Ipv4Addr::LOCALHOST);
        assert_eq!(addr.port(), 12345);
    }
}
