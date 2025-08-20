use std::{
    error::Error,
    future::Future,
    io::IoSlice,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::Context as _;
use http::{header::HOST, Request, Uri};
use http_body_util::BodyExt;
use hyper_util::{
    client::legacy::{
        connect::{Connected, Connection},
        Client,
    },
    rt::{TokioExecutor, TokioIo},
};
use spin_factor_outbound_networking::{
    config::{allowed_hosts::OutboundAllowedHosts, blocked_networks::BlockedNetworks},
    ComponentTlsClientConfigs, TlsClientConfig,
};
use spin_factors::{wasmtime::component::ResourceTable, RuntimeFactorsInstanceState};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::client::TlsStream;
use tower_service::Service;
use tracing::{field::Empty, instrument, Instrument};
use wasmtime::component::HasData;
use wasmtime_wasi::p2::{IoImpl, IoView};
use wasmtime_wasi_http::{
    bindings::http::types::ErrorCode,
    body::HyperOutgoingBody,
    types::{HostFutureIncomingResponse, IncomingResponse},
    WasiHttpCtx, WasiHttpImpl, WasiHttpView,
};

use crate::{
    intercept::{InterceptOutcome, OutboundHttpInterceptor},
    wasi_2023_10_18, wasi_2023_11_10, InstanceState, OutboundHttpFactor, SelfRequestOrigin,
};

pub(crate) struct HasHttp;

impl HasData for HasHttp {
    type Data<'a> = WasiHttpImpl<WasiHttpImplInner<'a>>;
}

pub(crate) fn add_to_linker<C>(ctx: &mut C) -> anyhow::Result<()>
where
    C: spin_factors::InitContext<OutboundHttpFactor>,
{
    fn get_http<C>(store: &mut C::StoreData) -> WasiHttpImpl<WasiHttpImplInner<'_>>
    where
        C: spin_factors::InitContext<OutboundHttpFactor>,
    {
        let (state, table) = C::get_data_with_table(store);
        WasiHttpImpl(IoImpl(WasiHttpImplInner { state, table }))
    }
    let get_http = get_http::<C> as fn(&mut C::StoreData) -> WasiHttpImpl<WasiHttpImplInner<'_>>;
    let linker = ctx.linker();
    wasmtime_wasi_http::bindings::http::outgoing_handler::add_to_linker::<_, HasHttp>(
        linker, get_http,
    )?;
    wasmtime_wasi_http::bindings::http::types::add_to_linker::<_, HasHttp>(
        linker,
        &Default::default(),
        get_http,
    )?;

    wasi_2023_10_18::add_to_linker(linker, get_http)?;
    wasi_2023_11_10::add_to_linker(linker, get_http)?;

    Ok(())
}

impl OutboundHttpFactor {
    pub fn get_wasi_http_impl(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
    ) -> Option<WasiHttpImpl<impl WasiHttpView + '_>> {
        let (state, table) = runtime_instance_state.get_with_table::<OutboundHttpFactor>()?;
        Some(WasiHttpImpl(IoImpl(WasiHttpImplInner { state, table })))
    }
}

type HttpClient = Client<HttpConnector, HyperOutgoingBody>;
type HttpsClient = Client<HttpsConnector, HyperOutgoingBody>;

#[derive(Clone)]
pub(super) struct HttpClients {
    /// Used for non-TLS HTTP/1 connections.
    http1: HttpClient,
    /// Used for non-TLS HTTP/2 connections (e.g. when h2 prior knowledge is available).
    http2: HttpClient,
    /// Used for HTTP-over-TLS connections, using ALPN to negotiate the HTTP version.
    https: HttpsClient,
}

impl HttpClients {
    pub(super) fn new(enable_pooling: bool) -> Self {
        let builder = move || {
            let mut builder = Client::builder(TokioExecutor::new());
            if !enable_pooling {
                builder.pool_max_idle_per_host(0);
            }
            builder
        };
        Self {
            http1: builder().build(HttpConnector),
            http2: builder().http2_only(true).build(HttpConnector),
            https: builder().build(HttpsConnector),
        }
    }
}

pub(crate) struct WasiHttpImplInner<'a> {
    state: &'a mut InstanceState,
    table: &'a mut ResourceTable,
}

impl IoView for WasiHttpImplInner<'_> {
    fn table(&mut self) -> &mut ResourceTable {
        self.table
    }
}

impl WasiHttpView for WasiHttpImplInner<'_> {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.state.wasi_http_ctx
    }

    #[instrument(
        name = "spin_outbound_http.send_request",
        skip_all,
        fields(
            otel.kind = "client",
            url.full = Empty,
            http.request.method = %request.method(),
            otel.name = %request.method(),
            http.response.status_code = Empty,
            server.address = Empty,
            server.port = Empty,
        ),
    )]
    fn send_request(
        &mut self,
        request: Request<wasmtime_wasi_http::body::HyperOutgoingBody>,
        config: wasmtime_wasi_http::types::OutgoingRequestConfig,
    ) -> wasmtime_wasi_http::HttpResult<wasmtime_wasi_http::types::HostFutureIncomingResponse> {
        Ok(HostFutureIncomingResponse::Pending(
            wasmtime_wasi::runtime::spawn(
                send_request_impl(
                    request,
                    config,
                    self.state.allowed_hosts.clone(),
                    self.state.component_tls_configs.clone(),
                    self.state.request_interceptor.clone(),
                    self.state.self_request_origin.clone(),
                    self.state.blocked_networks.clone(),
                    self.state.wasi_http_clients.clone(),
                )
                .in_current_span(),
            ),
        ))
    }
}

#[derive(Clone)]
struct ConnectOptions {
    blocked_networks: BlockedNetworks,
    connect_timeout: Duration,
}

// We must use task-local variables for these config options when using
// `hyper_util::client::legacy::Client::request` because there's no way to plumb
// them through as parameters.  Moreover, if there's already a pooled connection
// ready, we'll reuse that and ignore these options anyway.
tokio::task_local! {
    static CONNECT_OPTIONS: ConnectOptions;
    static TLS_CLIENT_CONFIG: TlsClientConfig;
}

async fn connect_tcp(uri: Uri, default_port: u16) -> Result<(TcpStream, String), ErrorCode> {
    let authority_str = if let Some(authority) = uri.authority() {
        if authority.port().is_some() {
            authority.to_string()
        } else {
            format!("{authority}:{default_port}")
        }
    } else {
        return Err(ErrorCode::HttpRequestUriInvalid);
    };

    let ConnectOptions {
        blocked_networks,
        connect_timeout,
    } = CONNECT_OPTIONS.get();

    let mut socket_addrs = tokio::net::lookup_host(&authority_str)
        .await
        .map_err(|_| dns_error("address not available".into(), 0))?
        .collect::<Vec<_>>();

    // Remove blocked IPs
    let blocked_addrs = blocked_networks.remove_blocked(&mut socket_addrs);
    if socket_addrs.is_empty() && !blocked_addrs.is_empty() {
        tracing::error!(
            "error.type" = "destination_ip_prohibited",
            ?blocked_addrs,
            "all destination IP(s) prohibited by runtime config"
        );
        return Err(ErrorCode::DestinationIpProhibited);
    }

    Ok((
        timeout(connect_timeout, TcpStream::connect(socket_addrs.as_slice()))
            .await
            .map_err(|_| ErrorCode::ConnectionTimeout)?
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::AddrNotAvailable => {
                    dns_error("address not available".into(), 0)
                }
                _ => ErrorCode::ConnectionRefused,
            })?,
        authority_str,
    ))
}

#[derive(Clone)]
struct HttpConnector;

impl HttpConnector {
    async fn connect(uri: Uri) -> Result<TokioIo<TcpStream>, ErrorCode> {
        Ok(TokioIo::new(connect_tcp(uri, 80).await?.0))
    }
}

impl Service<Uri> for HttpConnector {
    type Response = TokioIo<TcpStream>;
    type Error = ErrorCode;
    type Future = Pin<Box<dyn Future<Output = Result<TokioIo<TcpStream>, ErrorCode>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move { Self::connect(uri).await })
    }
}

struct RustlsStream(TlsStream<TcpStream>);

impl Connection for RustlsStream {
    fn connected(&self) -> Connected {
        if self.0.get_ref().1.alpn_protocol() == Some(b"h2") {
            self.0.get_ref().0.connected().negotiated_h2()
        } else {
            self.0.get_ref().0.connected()
        }
    }
}

impl AsyncRead for RustlsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for RustlsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }
}

#[derive(Clone)]
struct HttpsConnector;

impl HttpsConnector {
    async fn connect(uri: Uri) -> Result<TokioIo<RustlsStream>, ErrorCode> {
        use rustls::pki_types::ServerName;

        let (tcp_stream, authority_str) = connect_tcp(uri, 443).await?;

        let mut tls_client_config = (*TLS_CLIENT_CONFIG.get()).clone();
        tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_client_config));
        let mut parts = authority_str.split(':');
        let host = parts.next().unwrap_or(&authority_str);
        let domain = ServerName::try_from(host)
            .map_err(|e| {
                tracing::warn!("dns lookup error: {e:?}");
                dns_error("invalid dns name".to_string(), 0)
            })?
            .to_owned();
        let stream = connector.connect(domain, tcp_stream).await.map_err(|e| {
            tracing::warn!("tls protocol error: {e:?}");
            ErrorCode::TlsProtocolError
        })?;

        Ok(TokioIo::new(RustlsStream(stream)))
    }
}

impl Service<Uri> for HttpsConnector {
    type Response = TokioIo<RustlsStream>;
    type Error = ErrorCode;
    type Future = Pin<Box<dyn Future<Output = Result<TokioIo<RustlsStream>, ErrorCode>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move { Self::connect(uri).await })
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_request_impl(
    mut request: Request<wasmtime_wasi_http::body::HyperOutgoingBody>,
    mut config: wasmtime_wasi_http::types::OutgoingRequestConfig,
    outbound_allowed_hosts: OutboundAllowedHosts,
    component_tls_configs: ComponentTlsClientConfigs,
    request_interceptor: Option<Arc<dyn OutboundHttpInterceptor>>,
    self_request_origin: Option<SelfRequestOrigin>,
    blocked_networks: BlockedNetworks,
    http_clients: HttpClients,
) -> anyhow::Result<Result<IncomingResponse, ErrorCode>> {
    // wasmtime-wasi-http fills in scheme and authority for relative URLs
    // (e.g. https://:443/<path>), which makes them hard to reason about.
    // Undo that here.
    let uri = request.uri_mut();
    if uri
        .authority()
        .is_some_and(|authority| authority.host().is_empty())
    {
        let mut builder = http::uri::Builder::new();
        if let Some(paq) = uri.path_and_query() {
            builder = builder.path_and_query(paq.clone());
        }
        *uri = builder.build().unwrap();
    }
    let span = tracing::Span::current();
    span.record("url.full", uri.to_string());

    spin_telemetry::inject_trace_context(&mut request);

    let host = request.uri().host().unwrap_or_default();
    let tls_client_config = component_tls_configs.get_client_config(host).clone();

    let is_self_request = request
        .uri()
        .authority()
        .is_some_and(|a| a.host() == "self.alt");

    if request.uri().authority().is_some() && !is_self_request {
        // Absolute URI
        let is_allowed = outbound_allowed_hosts
            .check_url(&request.uri().to_string(), "https")
            .await
            .unwrap_or(false);
        if !is_allowed {
            return Ok(Err(ErrorCode::HttpRequestDenied));
        }
    } else {
        // Relative URI ("self" request)
        let is_allowed = outbound_allowed_hosts
            .check_relative_url(&["http", "https"])
            .await
            .unwrap_or(false);
        if !is_allowed {
            return Ok(Err(ErrorCode::HttpRequestDenied));
        }

        let Some(origin) = self_request_origin else {
            tracing::error!("Couldn't handle outbound HTTP request to relative URI; no origin set");
            return Ok(Err(ErrorCode::HttpRequestUriInvalid));
        };

        config.use_tls = origin.use_tls();

        request.headers_mut().insert(HOST, origin.host_header());

        let path_and_query = request.uri().path_and_query().cloned();
        *request.uri_mut() = origin.into_uri(path_and_query);
    }

    // Some servers (looking at you nginx) don't like a host header even though
    // http/2 allows it: https://github.com/hyperium/hyper/issues/3298.
    //
    // Note that we do this _before_ invoking the request interceptor.  It may
    // decide to add the `host` header back in, regardless of the nginx bug, in
    // which case we'll let it do so without interferring.
    request.headers_mut().remove(HOST);

    if let Some(interceptor) = request_interceptor {
        let intercept_request = std::mem::take(&mut request).into();
        match interceptor.intercept(intercept_request).await? {
            InterceptOutcome::Continue(req) => {
                request = req.into_hyper_request();
            }
            InterceptOutcome::Complete(resp) => {
                let resp = IncomingResponse {
                    resp,
                    worker: None,
                    between_bytes_timeout: config.between_bytes_timeout,
                };
                return Ok(Ok(resp));
            }
        }
    }

    let authority = request.uri().authority().context("authority not set")?;
    span.record("server.address", authority.host());
    if let Some(port) = authority.port() {
        span.record("server.port", port.as_u16());
    }

    Ok(send_request_handler(
        request,
        config,
        tls_client_config,
        blocked_networks,
        http_clients,
    )
    .await)
}

async fn send_request_handler(
    request: http::Request<HyperOutgoingBody>,
    wasmtime_wasi_http::types::OutgoingRequestConfig {
        use_tls,
        connect_timeout,
        first_byte_timeout,
        between_bytes_timeout,
    }: wasmtime_wasi_http::types::OutgoingRequestConfig,
    tls_client_config: TlsClientConfig,
    blocked_networks: BlockedNetworks,
    http_clients: HttpClients,
) -> Result<wasmtime_wasi_http::types::IncomingResponse, ErrorCode> {
    let resp = CONNECT_OPTIONS.scope(
        ConnectOptions {
            blocked_networks,
            connect_timeout,
        },
        async move {
            if use_tls {
                TLS_CLIENT_CONFIG
                    .scope(tls_client_config, async move {
                        http_clients.https.request(request).await
                    })
                    .await
            } else {
                let use_http2 =
                    std::env::var_os("SPIN_OUTBOUND_H2C_PRIOR_KNOWLEDGE").is_some_and(|v| {
                        request
                            .uri()
                            .authority()
                            .is_some_and(|authority| authority.as_str() == v)
                    });

                if use_http2 {
                    http_clients.http2.request(request).await
                } else {
                    http_clients.http1.request(request).await
                }
            }
        },
    );

    let resp = timeout(first_byte_timeout, resp)
        .await
        .map_err(|_| ErrorCode::ConnectionReadTimeout)?
        .map_err(hyper_legacy_request_error)?
        .map(|body| body.map_err(hyper_request_error).boxed());

    tracing::Span::current().record("http.response.status_code", resp.status().as_u16());

    Ok(wasmtime_wasi_http::types::IncomingResponse {
        resp,
        worker: None,
        between_bytes_timeout,
    })
}

/// Translate a [`hyper::Error`] to a wasi-http `ErrorCode` in the context of a request.
fn hyper_request_error(err: hyper::Error) -> ErrorCode {
    // If there's a source, we might be able to extract a wasi-http error from it.
    if let Some(cause) = err.source() {
        if let Some(err) = cause.downcast_ref::<ErrorCode>() {
            return err.clone();
        }
    }

    tracing::warn!("hyper request error: {err:?}");

    ErrorCode::HttpProtocolError
}

/// Translate a [`hyper_util::client::legacy::Error`] to a wasi-http `ErrorCode` in the context of a request.
fn hyper_legacy_request_error(err: hyper_util::client::legacy::Error) -> ErrorCode {
    // If there's a source, we might be able to extract a wasi-http error from it.
    if let Some(cause) = err.source() {
        if let Some(err) = cause.downcast_ref::<ErrorCode>() {
            return err.clone();
        }
    }

    tracing::warn!("hyper request error: {err:?}");

    ErrorCode::HttpProtocolError
}

fn dns_error(rcode: String, info_code: u16) -> ErrorCode {
    ErrorCode::DnsError(wasmtime_wasi_http::bindings::http::types::DnsErrorPayload {
        rcode: Some(rcode),
        info_code: Some(info_code),
    })
}
