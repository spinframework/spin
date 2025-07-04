//! Implementation for the Spin HTTP engine.

mod headers;
mod instrument;
mod outbound_http;
mod server;
mod spin;
mod tls;
mod wagi;
mod wasi;

use std::{
    error::Error,
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
    path::PathBuf,
    sync::Arc,
};

use anyhow::{bail, Context};
use clap::Args;
use serde::Deserialize;
use spin_app::App;
use spin_factors::RuntimeFactors;
use spin_trigger::Trigger;
use wasmtime_wasi_http::bindings::http::types::ErrorCode;

pub use server::HttpServer;

pub use tls::TlsConfig;

pub(crate) use wasmtime_wasi_http::body::HyperIncomingBody as Body;

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

    #[clap(long = "find-free-port")]
    pub find_free_port: bool,
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

/// The Spin HTTP trigger.
pub struct HttpTrigger {
    /// The address the server should listen on.
    ///
    /// Note that this might not be the actual socket address that ends up being bound to.
    /// If the port is set to 0, the actual address will be determined by the OS.
    listen_addr: SocketAddr,
    tls_config: Option<TlsConfig>,
    find_free_port: bool,
}

impl<F: RuntimeFactors> Trigger<F> for HttpTrigger {
    const TYPE: &'static str = "http";

    type CliArgs = CliArgs;
    type InstanceState = ();

    fn new(cli_args: Self::CliArgs, app: &spin_app::App) -> anyhow::Result<Self> {
        let find_free_port = cli_args.find_free_port;

        Self::new(
            app,
            cli_args.address,
            cli_args.into_tls_config(),
            find_free_port,
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
    ) -> anyhow::Result<Self> {
        Self::validate_app(app)?;

        Ok(Self {
            listen_addr,
            tls_config,
            find_free_port,
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
        } = self;
        let server = Arc::new(HttpServer::new(
            listen_addr,
            tls_config,
            find_free_port,
            trigger_app,
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
                tracing::warn!("This application has the deprecated trigger 'base' set to the default value '/'. This may be an error in the future!");
            } else {
                bail!("This application is using the deprecated trigger 'base' field. The base must be prepended to each [[trigger.http]]'s 'route'.")
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
