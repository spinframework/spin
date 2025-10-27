use std::{
    error::Error,
    future::Future,
    io::IoSlice,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{self, Context, Poll},
    time::Duration,
};

use bytes::Bytes;
use http::{header::HOST, uri::Scheme, Uri};
use http_body::{Body, Frame, SizeHint};
use http_body_util::{combinators::BoxBody, BodyExt};
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
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use tokio_rustls::client::TlsStream;
use tower_service::Service;
use tracing::{field::Empty, instrument, Instrument};
use wasmtime::component::HasData;
use wasmtime_wasi::TrappableError;
use wasmtime_wasi_http::{
    bindings::http::types::{self as p2_types, ErrorCode},
    body::HyperOutgoingBody,
    p3::{self, bindings::http::types as p3_types},
    types::{HostFutureIncomingResponse, IncomingResponse, OutgoingRequestConfig},
    HttpError, WasiHttpCtx, WasiHttpImpl, WasiHttpView,
};

use crate::{
    intercept::{InterceptOutcome, OutboundHttpInterceptor},
    wasi_2023_10_18, wasi_2023_11_10, InstanceState, OutboundHttpFactor, SelfRequestOrigin,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct HasHttp;

impl HasData for HasHttp {
    type Data<'a> = WasiHttpImpl<WasiHttpImplInner<'a>>;
}

impl p3::WasiHttpCtx for InstanceState {
    fn send_request(
        &mut self,
        request: http::Request<BoxBody<Bytes, p3_types::ErrorCode>>,
        options: Option<p3::RequestOptions>,
        fut: Box<dyn Future<Output = Result<(), p3_types::ErrorCode>> + Send>,
    ) -> Box<
        dyn Future<
                Output = Result<
                    (
                        http::Response<BoxBody<Bytes, p3_types::ErrorCode>>,
                        Box<dyn Future<Output = Result<(), p3_types::ErrorCode>> + Send>,
                    ),
                    TrappableError<p3_types::ErrorCode>,
                >,
            > + Send,
    > {
        // If the caller (i.e. the guest) has trouble consuming the response
        // (e.g. encountering a network error while forwarding it on to some
        // other place), it can report that error to us via `fut`.  However,
        // there's nothing we'll be able to do with it here, so we ignore it.
        // Presumably the guest will also drop the body stream and trailers
        // future if it encounters such an error while those things are still
        // arriving, which Hyper will deal with as appropriate (e.g. closing the
        // connection).
        _ = fut;

        let request_sender = RequestSender {
            allowed_hosts: self.allowed_hosts.clone(),
            component_tls_configs: self.component_tls_configs.clone(),
            request_interceptor: self.request_interceptor.clone(),
            self_request_origin: self.self_request_origin.clone(),
            blocked_networks: self.blocked_networks.clone(),
            http_clients: self.wasi_http_clients.clone(),
            concurrent_outbound_connections_semaphore: self
                .concurrent_outbound_connections_semaphore
                .clone(),
        };
        let config = OutgoingRequestConfig {
            use_tls: request.uri().scheme() == Some(&Scheme::HTTPS),
            connect_timeout: options
                .and_then(|v| v.connect_timeout)
                .unwrap_or(DEFAULT_TIMEOUT),
            first_byte_timeout: options
                .and_then(|v| v.first_byte_timeout)
                .unwrap_or(DEFAULT_TIMEOUT),
            between_bytes_timeout: options
                .and_then(|v| v.between_bytes_timeout)
                .unwrap_or(DEFAULT_TIMEOUT),
        };
        Box::new(async {
            match request_sender
                .send(
                    request.map(|body| body.map_err(p3_to_p2_error_code).boxed()),
                    config,
                )
                .await
            {
                Ok(IncomingResponse {
                    resp,
                    between_bytes_timeout,
                    ..
                }) => Ok((
                    resp.map(|body| {
                        BetweenBytesTimeoutBody {
                            body,
                            sleep: None,
                            timeout: between_bytes_timeout,
                        }
                        .boxed()
                    }),
                    Box::new(async {
                        // TODO: Can we plumb connection errors through to here, or
                        // will `hyper_util::client::legacy::Client` pass them all
                        // via the response body?
                        Ok(())
                    }) as Box<dyn Future<Output = _> + Send>,
                )),
                Err(http_error) => match http_error.downcast() {
                    Ok(error_code) => Err(TrappableError::from(p2_to_p3_error_code(error_code))),
                    Err(trap) => Err(TrappableError::trap(trap)),
                },
            }
        })
    }
}

pin_project_lite::pin_project! {
    struct BetweenBytesTimeoutBody<B> {
        #[pin]
        body: B,
        #[pin]
        sleep: Option<tokio::time::Sleep>,
        timeout: Duration,
    }
}

impl<B: Body<Error = p2_types::ErrorCode>> Body for BetweenBytesTimeoutBody<B> {
    type Data = B::Data;
    type Error = p3_types::ErrorCode;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let mut me = self.project();
        match me.body.poll_frame(cx) {
            Poll::Ready(value) => {
                me.sleep.as_mut().set(None);
                Poll::Ready(value.map(|v| v.map_err(p2_to_p3_error_code)))
            }
            Poll::Pending => {
                if me.sleep.is_none() {
                    me.sleep.as_mut().set(Some(tokio::time::sleep(*me.timeout)));
                }
                task::ready!(me.sleep.as_pin_mut().unwrap().poll(cx));
                Poll::Ready(Some(Err(p3_types::ErrorCode::ConnectionReadTimeout)))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

pub(crate) fn add_to_linker<C>(ctx: &mut C) -> anyhow::Result<()>
where
    C: spin_factors::InitContext<OutboundHttpFactor>,
{
    let linker = ctx.linker();

    fn get_http<C>(store: &mut C::StoreData) -> WasiHttpImpl<WasiHttpImplInner<'_>>
    where
        C: spin_factors::InitContext<OutboundHttpFactor>,
    {
        let (state, table) = C::get_data_with_table(store);
        WasiHttpImpl(WasiHttpImplInner { state, table })
    }

    let get_http = get_http::<C> as fn(&mut C::StoreData) -> WasiHttpImpl<WasiHttpImplInner<'_>>;
    wasmtime_wasi_http::bindings::http::outgoing_handler::add_to_linker::<_, HasHttp>(
        linker, get_http,
    )?;
    wasmtime_wasi_http::bindings::http::types::add_to_linker::<_, HasHttp>(
        linker,
        &Default::default(),
        get_http,
    )?;

    fn get_http_p3<C>(store: &mut C::StoreData) -> p3::WasiHttpCtxView<'_>
    where
        C: spin_factors::InitContext<OutboundHttpFactor>,
    {
        let (state, table) = C::get_data_with_table(store);
        p3::WasiHttpCtxView { ctx: state, table }
    }

    let get_http_p3 = get_http_p3::<C> as fn(&mut C::StoreData) -> p3::WasiHttpCtxView<'_>;
    p3::bindings::http::handler::add_to_linker::<_, p3::WasiHttp>(linker, get_http_p3)?;
    p3::bindings::http::types::add_to_linker::<_, p3::WasiHttp>(linker, get_http_p3)?;

    wasi_2023_10_18::add_to_linker(linker, get_http)?;
    wasi_2023_11_10::add_to_linker(linker, get_http)?;

    Ok(())
}

impl OutboundHttpFactor {
    pub fn get_wasi_http_impl(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
    ) -> Option<WasiHttpImpl<impl WasiHttpView + '_>> {
        let (state, table) = runtime_instance_state.get_with_table::<OutboundHttpFactor>()?;
        Some(WasiHttpImpl(WasiHttpImplInner { state, table }))
    }

    pub fn get_wasi_p3_http_impl(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
    ) -> Option<p3::WasiHttpCtxView<'_>> {
        let (state, table) = runtime_instance_state.get_with_table::<OutboundHttpFactor>()?;
        Some(p3::WasiHttpCtxView { ctx: state, table })
    }
}

pub(crate) struct WasiHttpImplInner<'a> {
    state: &'a mut InstanceState,
    table: &'a mut ResourceTable,
}

type OutgoingRequest = http::Request<HyperOutgoingBody>;

impl WasiHttpView for WasiHttpImplInner<'_> {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.state.wasi_http_ctx
    }

    fn table(&mut self) -> &mut ResourceTable {
        self.table
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
        )
    )]
    fn send_request(
        &mut self,
        request: OutgoingRequest,
        config: OutgoingRequestConfig,
    ) -> Result<wasmtime_wasi_http::types::HostFutureIncomingResponse, HttpError> {
        let request_sender = RequestSender {
            allowed_hosts: self.state.allowed_hosts.clone(),
            component_tls_configs: self.state.component_tls_configs.clone(),
            request_interceptor: self.state.request_interceptor.clone(),
            self_request_origin: self.state.self_request_origin.clone(),
            blocked_networks: self.state.blocked_networks.clone(),
            http_clients: self.state.wasi_http_clients.clone(),
            concurrent_outbound_connections_semaphore: self
                .state
                .concurrent_outbound_connections_semaphore
                .clone(),
        };
        Ok(HostFutureIncomingResponse::Pending(
            wasmtime_wasi::runtime::spawn(
                async {
                    match request_sender.send(request, config).await {
                        Ok(resp) => Ok(Ok(resp)),
                        Err(http_error) => match http_error.downcast() {
                            Ok(error_code) => Ok(Err(error_code)),
                            Err(trap) => Err(trap),
                        },
                    }
                }
                .in_current_span(),
            ),
        ))
    }
}

struct RequestSender {
    allowed_hosts: OutboundAllowedHosts,
    blocked_networks: BlockedNetworks,
    component_tls_configs: ComponentTlsClientConfigs,
    self_request_origin: Option<SelfRequestOrigin>,
    request_interceptor: Option<Arc<dyn OutboundHttpInterceptor>>,
    http_clients: HttpClients,
    concurrent_outbound_connections_semaphore: Option<Arc<Semaphore>>,
}

impl RequestSender {
    async fn send(
        self,
        mut request: OutgoingRequest,
        mut config: OutgoingRequestConfig,
    ) -> Result<IncomingResponse, HttpError> {
        self.prepare_request(&mut request, &mut config).await?;

        // If the current span has opentelemetry trace context, inject it into the request
        spin_telemetry::inject_trace_context(&mut request);

        // Run any configured request interceptor
        let mut override_connect_addr = None;
        if let Some(interceptor) = &self.request_interceptor {
            let intercept_request = std::mem::take(&mut request).into();
            match interceptor.intercept(intercept_request).await? {
                InterceptOutcome::Continue(mut req) => {
                    override_connect_addr = req.override_connect_addr.take();
                    request = req.into_hyper_request();
                }
                InterceptOutcome::Complete(resp) => {
                    let resp = IncomingResponse {
                        resp,
                        worker: None,
                        between_bytes_timeout: config.between_bytes_timeout,
                    };
                    return Ok(resp);
                }
            }
        }

        // Backfill span fields after potentially updating the URL in the interceptor
        let span = tracing::Span::current();
        if let Some(addr) = override_connect_addr {
            span.record("server.address", addr.ip().to_string());
            span.record("server.port", addr.port());
        } else if let Some(authority) = request.uri().authority() {
            span.record("server.address", authority.host());
            if let Some(port) = authority.port_u16() {
                span.record("server.port", port);
            }
        }

        Ok(self
            .send_request(request, config, override_connect_addr)
            .await?)
    }

    async fn prepare_request(
        &self,
        request: &mut OutgoingRequest,
        config: &mut OutgoingRequestConfig,
    ) -> Result<(), ErrorCode> {
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
        tracing::Span::current().record("url.full", uri.to_string());

        let is_self_request = match request.uri().authority() {
            // Some SDKs require an authority, so we support e.g. http://self.alt/self-request
            Some(authority) => authority.host() == "self.alt",
            // Otherwise self requests have no authority
            None => true,
        };

        // Enforce allowed_outbound_hosts
        let is_allowed = if is_self_request {
            self.allowed_hosts
                .check_relative_url(&["http", "https"])
                .await
                .unwrap_or(false)
        } else {
            self.allowed_hosts
                .check_url(&request.uri().to_string(), "https")
                .await
                .unwrap_or(false)
        };
        if !is_allowed {
            return Err(ErrorCode::HttpRequestDenied);
        }

        if is_self_request {
            // Replace the authority with the "self request origin"
            let Some(origin) = self.self_request_origin.as_ref() else {
                tracing::error!(
                    "Couldn't handle outbound HTTP request to relative URI; no origin set"
                );
                return Err(ErrorCode::HttpRequestUriInvalid);
            };

            config.use_tls = origin.use_tls();

            request.headers_mut().insert(HOST, origin.host_header());

            let path_and_query = request.uri().path_and_query().cloned();
            *request.uri_mut() = origin.clone().into_uri(path_and_query);
        }

        // Some servers (looking at you nginx) don't like a host header even though
        // http/2 allows it: https://github.com/hyperium/hyper/issues/3298.
        //
        // Note that we do this _before_ invoking the request interceptor.  It may
        // decide to add the `host` header back in, regardless of the nginx bug, in
        // which case we'll let it do so without interferring.
        request.headers_mut().remove(HOST);
        Ok(())
    }

    async fn send_request(
        self,
        request: OutgoingRequest,
        config: OutgoingRequestConfig,
        override_connect_addr: Option<SocketAddr>,
    ) -> Result<IncomingResponse, ErrorCode> {
        let OutgoingRequestConfig {
            use_tls,
            connect_timeout,
            first_byte_timeout,
            between_bytes_timeout,
        } = config;

        let tls_client_config = if use_tls {
            let host = request.uri().host().unwrap_or_default();
            Some(self.component_tls_configs.get_client_config(host).clone())
        } else {
            None
        };

        let resp = CONNECT_OPTIONS.scope(
            ConnectOptions {
                blocked_networks: self.blocked_networks,
                connect_timeout,
                tls_client_config,
                override_connect_addr,
                concurrent_outbound_connections_semaphore: self
                    .concurrent_outbound_connections_semaphore,
            },
            async move {
                if use_tls {
                    self.http_clients.https.request(request).await
                } else {
                    // For development purposes, allow configuring plaintext HTTP/2 for a specific host.
                    let h2c_prior_knowledge_host =
                        std::env::var("SPIN_OUTBOUND_H2C_PRIOR_KNOWLEDGE").ok();
                    let use_h2c = h2c_prior_knowledge_host.as_deref()
                        == request.uri().authority().map(|a| a.as_str());

                    if use_h2c {
                        self.http_clients.http2.request(request).await
                    } else {
                        self.http_clients.http1.request(request).await
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

        Ok(IncomingResponse {
            resp,
            worker: None,
            between_bytes_timeout,
        })
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

tokio::task_local! {
    /// The options used when establishing a new connection.
    ///
    /// We must use task-local variables for these config options when using
    /// `hyper_util::client::legacy::Client::request` because there's no way to plumb
    /// them through as parameters.  Moreover, if there's already a pooled connection
    /// ready, we'll reuse that and ignore these options anyway. After each connection
    /// is established, the options are dropped.
    static CONNECT_OPTIONS: ConnectOptions;
}

#[derive(Clone)]
struct ConnectOptions {
    /// The blocked networks configuration.
    blocked_networks: BlockedNetworks,
    /// Timeout for establishing a TCP connection.
    connect_timeout: Duration,
    /// TLS client configuration to use, if any.
    tls_client_config: Option<TlsClientConfig>,
    /// If set, override the address to connect to instead of using the given `uri`'s authority.
    override_connect_addr: Option<SocketAddr>,
    /// A semaphore to limit the number of concurrent outbound connections.
    concurrent_outbound_connections_semaphore: Option<Arc<Semaphore>>,
}

impl ConnectOptions {
    /// Establish a TCP connection to the given URI and default port.
    async fn connect_tcp(
        &self,
        uri: &Uri,
        default_port: u16,
    ) -> Result<PermittedTcpStream, ErrorCode> {
        let mut socket_addrs = match self.override_connect_addr {
            Some(override_connect_addr) => vec![override_connect_addr],
            None => {
                let authority = uri.authority().ok_or(ErrorCode::HttpRequestUriInvalid)?;

                let host_and_port = if authority.port().is_some() {
                    authority.as_str().to_string()
                } else {
                    format!("{}:{}", authority.as_str(), default_port)
                };

                let socket_addrs = tokio::net::lookup_host(&host_and_port)
                    .await
                    .map_err(|err| {
                        tracing::debug!(?host_and_port, ?err, "Error resolving host");
                        dns_error("address not available".into(), 0)
                    })?
                    .collect::<Vec<_>>();
                tracing::debug!(?host_and_port, ?socket_addrs, "Resolved host");
                socket_addrs
            }
        };

        // Remove blocked IPs
        crate::remove_blocked_addrs(&self.blocked_networks, &mut socket_addrs)?;

        // If we're limiting concurrent outbound requests, acquire a permit
        let permit = match &self.concurrent_outbound_connections_semaphore {
            Some(s) => s.clone().acquire_owned().await.ok(),
            None => None,
        };

        let stream = timeout(self.connect_timeout, TcpStream::connect(&*socket_addrs))
            .await
            .map_err(|_| ErrorCode::ConnectionTimeout)?
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::AddrNotAvailable => {
                    dns_error("address not available".into(), 0)
                }
                _ => ErrorCode::ConnectionRefused,
            })?;
        Ok(PermittedTcpStream {
            inner: stream,
            _permit: permit,
        })
    }

    /// Establish a TLS connection to the given URI and default port.
    async fn connect_tls(
        &self,
        uri: &Uri,
        default_port: u16,
    ) -> Result<TlsStream<PermittedTcpStream>, ErrorCode> {
        let tcp_stream = self.connect_tcp(uri, default_port).await?;

        let mut tls_client_config = self.tls_client_config.as_deref().unwrap().clone();
        tls_client_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_client_config));
        let domain = rustls::pki_types::ServerName::try_from(uri.host().unwrap())
            .map_err(|e| {
                tracing::warn!("dns lookup error: {e:?}");
                dns_error("invalid dns name".into(), 0)
            })?
            .to_owned();
        connector.connect(domain, tcp_stream).await.map_err(|e| {
            tracing::warn!("tls protocol error: {e:?}");
            ErrorCode::TlsProtocolError
        })
    }
}

/// A connector the uses `ConnectOptions`
#[derive(Clone)]
struct HttpConnector;

impl HttpConnector {
    async fn connect(uri: Uri) -> Result<TokioIo<PermittedTcpStream>, ErrorCode> {
        let stream = CONNECT_OPTIONS.get().connect_tcp(&uri, 80).await?;
        Ok(TokioIo::new(stream))
    }
}

impl Service<Uri> for HttpConnector {
    type Response = TokioIo<PermittedTcpStream>;
    type Error = ErrorCode;
    type Future =
        Pin<Box<dyn Future<Output = Result<TokioIo<PermittedTcpStream>, ErrorCode>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        Box::pin(async move { Self::connect(uri).await })
    }
}

/// A connector that establishes TLS connections using `rustls` and `ConnectOptions`.
#[derive(Clone)]
struct HttpsConnector;

impl HttpsConnector {
    async fn connect(uri: Uri) -> Result<TokioIo<RustlsStream>, ErrorCode> {
        let stream = CONNECT_OPTIONS.get().connect_tls(&uri, 443).await?;
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

struct RustlsStream(TlsStream<PermittedTcpStream>);

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

/// A TCP stream that holds an optional permit indicating that it is allowed to exist.
struct PermittedTcpStream {
    /// The wrapped TCP stream.
    inner: TcpStream,
    /// A permit indicating that this stream is allowed to exist.
    ///
    /// When this stream is dropped, the permit is also dropped, allowing another
    /// connection to be established.
    _permit: Option<OwnedSemaphorePermit>,
}

impl Connection for PermittedTcpStream {
    fn connected(&self) -> Connected {
        self.inner.connected()
    }
}

impl AsyncRead for PermittedTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PermittedTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
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

// TODO: Remove this (and uses of it) once
// https://github.com/spinframework/spin/issues/3274 has been addressed.
pub fn p2_to_p3_error_code(code: p2_types::ErrorCode) -> p3_types::ErrorCode {
    match code {
        p2_types::ErrorCode::DnsTimeout => p3_types::ErrorCode::DnsTimeout,
        p2_types::ErrorCode::DnsError(payload) => {
            p3_types::ErrorCode::DnsError(p3_types::DnsErrorPayload {
                rcode: payload.rcode,
                info_code: payload.info_code,
            })
        }
        p2_types::ErrorCode::DestinationNotFound => p3_types::ErrorCode::DestinationNotFound,
        p2_types::ErrorCode::DestinationUnavailable => p3_types::ErrorCode::DestinationUnavailable,
        p2_types::ErrorCode::DestinationIpProhibited => {
            p3_types::ErrorCode::DestinationIpProhibited
        }
        p2_types::ErrorCode::DestinationIpUnroutable => {
            p3_types::ErrorCode::DestinationIpUnroutable
        }
        p2_types::ErrorCode::ConnectionRefused => p3_types::ErrorCode::ConnectionRefused,
        p2_types::ErrorCode::ConnectionTerminated => p3_types::ErrorCode::ConnectionTerminated,
        p2_types::ErrorCode::ConnectionTimeout => p3_types::ErrorCode::ConnectionTimeout,
        p2_types::ErrorCode::ConnectionReadTimeout => p3_types::ErrorCode::ConnectionReadTimeout,
        p2_types::ErrorCode::ConnectionWriteTimeout => p3_types::ErrorCode::ConnectionWriteTimeout,
        p2_types::ErrorCode::ConnectionLimitReached => p3_types::ErrorCode::ConnectionLimitReached,
        p2_types::ErrorCode::TlsProtocolError => p3_types::ErrorCode::TlsProtocolError,
        p2_types::ErrorCode::TlsCertificateError => p3_types::ErrorCode::TlsCertificateError,
        p2_types::ErrorCode::TlsAlertReceived(payload) => {
            p3_types::ErrorCode::TlsAlertReceived(p3_types::TlsAlertReceivedPayload {
                alert_id: payload.alert_id,
                alert_message: payload.alert_message,
            })
        }
        p2_types::ErrorCode::HttpRequestDenied => p3_types::ErrorCode::HttpRequestDenied,
        p2_types::ErrorCode::HttpRequestLengthRequired => {
            p3_types::ErrorCode::HttpRequestLengthRequired
        }
        p2_types::ErrorCode::HttpRequestBodySize(payload) => {
            p3_types::ErrorCode::HttpRequestBodySize(payload)
        }
        p2_types::ErrorCode::HttpRequestMethodInvalid => {
            p3_types::ErrorCode::HttpRequestMethodInvalid
        }
        p2_types::ErrorCode::HttpRequestUriInvalid => p3_types::ErrorCode::HttpRequestUriInvalid,
        p2_types::ErrorCode::HttpRequestUriTooLong => p3_types::ErrorCode::HttpRequestUriTooLong,
        p2_types::ErrorCode::HttpRequestHeaderSectionSize(payload) => {
            p3_types::ErrorCode::HttpRequestHeaderSectionSize(payload)
        }
        p2_types::ErrorCode::HttpRequestHeaderSize(payload) => {
            p3_types::ErrorCode::HttpRequestHeaderSize(payload.map(|payload| {
                p3_types::FieldSizePayload {
                    field_name: payload.field_name,
                    field_size: payload.field_size,
                }
            }))
        }
        p2_types::ErrorCode::HttpRequestTrailerSectionSize(payload) => {
            p3_types::ErrorCode::HttpRequestTrailerSectionSize(payload)
        }
        p2_types::ErrorCode::HttpRequestTrailerSize(payload) => {
            p3_types::ErrorCode::HttpRequestTrailerSize(p3_types::FieldSizePayload {
                field_name: payload.field_name,
                field_size: payload.field_size,
            })
        }
        p2_types::ErrorCode::HttpResponseIncomplete => p3_types::ErrorCode::HttpResponseIncomplete,
        p2_types::ErrorCode::HttpResponseHeaderSectionSize(payload) => {
            p3_types::ErrorCode::HttpResponseHeaderSectionSize(payload)
        }
        p2_types::ErrorCode::HttpResponseHeaderSize(payload) => {
            p3_types::ErrorCode::HttpResponseHeaderSize(p3_types::FieldSizePayload {
                field_name: payload.field_name,
                field_size: payload.field_size,
            })
        }
        p2_types::ErrorCode::HttpResponseBodySize(payload) => {
            p3_types::ErrorCode::HttpResponseBodySize(payload)
        }
        p2_types::ErrorCode::HttpResponseTrailerSectionSize(payload) => {
            p3_types::ErrorCode::HttpResponseTrailerSectionSize(payload)
        }
        p2_types::ErrorCode::HttpResponseTrailerSize(payload) => {
            p3_types::ErrorCode::HttpResponseTrailerSize(p3_types::FieldSizePayload {
                field_name: payload.field_name,
                field_size: payload.field_size,
            })
        }
        p2_types::ErrorCode::HttpResponseTransferCoding(payload) => {
            p3_types::ErrorCode::HttpResponseTransferCoding(payload)
        }
        p2_types::ErrorCode::HttpResponseContentCoding(payload) => {
            p3_types::ErrorCode::HttpResponseContentCoding(payload)
        }
        p2_types::ErrorCode::HttpResponseTimeout => p3_types::ErrorCode::HttpResponseTimeout,
        p2_types::ErrorCode::HttpUpgradeFailed => p3_types::ErrorCode::HttpUpgradeFailed,
        p2_types::ErrorCode::HttpProtocolError => p3_types::ErrorCode::HttpProtocolError,
        p2_types::ErrorCode::LoopDetected => p3_types::ErrorCode::LoopDetected,
        p2_types::ErrorCode::ConfigurationError => p3_types::ErrorCode::ConfigurationError,
        p2_types::ErrorCode::InternalError(payload) => p3_types::ErrorCode::InternalError(payload),
    }
}

// TODO: Remove this (and uses of it) once
// https://github.com/spinframework/spin/issues/3274 has been addressed.
pub fn p3_to_p2_error_code(code: p3_types::ErrorCode) -> p2_types::ErrorCode {
    match code {
        p3_types::ErrorCode::DnsTimeout => p2_types::ErrorCode::DnsTimeout,
        p3_types::ErrorCode::DnsError(payload) => {
            p2_types::ErrorCode::DnsError(p2_types::DnsErrorPayload {
                rcode: payload.rcode,
                info_code: payload.info_code,
            })
        }
        p3_types::ErrorCode::DestinationNotFound => p2_types::ErrorCode::DestinationNotFound,
        p3_types::ErrorCode::DestinationUnavailable => p2_types::ErrorCode::DestinationUnavailable,
        p3_types::ErrorCode::DestinationIpProhibited => {
            p2_types::ErrorCode::DestinationIpProhibited
        }
        p3_types::ErrorCode::DestinationIpUnroutable => {
            p2_types::ErrorCode::DestinationIpUnroutable
        }
        p3_types::ErrorCode::ConnectionRefused => p2_types::ErrorCode::ConnectionRefused,
        p3_types::ErrorCode::ConnectionTerminated => p2_types::ErrorCode::ConnectionTerminated,
        p3_types::ErrorCode::ConnectionTimeout => p2_types::ErrorCode::ConnectionTimeout,
        p3_types::ErrorCode::ConnectionReadTimeout => p2_types::ErrorCode::ConnectionReadTimeout,
        p3_types::ErrorCode::ConnectionWriteTimeout => p2_types::ErrorCode::ConnectionWriteTimeout,
        p3_types::ErrorCode::ConnectionLimitReached => p2_types::ErrorCode::ConnectionLimitReached,
        p3_types::ErrorCode::TlsProtocolError => p2_types::ErrorCode::TlsProtocolError,
        p3_types::ErrorCode::TlsCertificateError => p2_types::ErrorCode::TlsCertificateError,
        p3_types::ErrorCode::TlsAlertReceived(payload) => {
            p2_types::ErrorCode::TlsAlertReceived(p2_types::TlsAlertReceivedPayload {
                alert_id: payload.alert_id,
                alert_message: payload.alert_message,
            })
        }
        p3_types::ErrorCode::HttpRequestDenied => p2_types::ErrorCode::HttpRequestDenied,
        p3_types::ErrorCode::HttpRequestLengthRequired => {
            p2_types::ErrorCode::HttpRequestLengthRequired
        }
        p3_types::ErrorCode::HttpRequestBodySize(payload) => {
            p2_types::ErrorCode::HttpRequestBodySize(payload)
        }
        p3_types::ErrorCode::HttpRequestMethodInvalid => {
            p2_types::ErrorCode::HttpRequestMethodInvalid
        }
        p3_types::ErrorCode::HttpRequestUriInvalid => p2_types::ErrorCode::HttpRequestUriInvalid,
        p3_types::ErrorCode::HttpRequestUriTooLong => p2_types::ErrorCode::HttpRequestUriTooLong,
        p3_types::ErrorCode::HttpRequestHeaderSectionSize(payload) => {
            p2_types::ErrorCode::HttpRequestHeaderSectionSize(payload)
        }
        p3_types::ErrorCode::HttpRequestHeaderSize(payload) => {
            p2_types::ErrorCode::HttpRequestHeaderSize(payload.map(|payload| {
                p2_types::FieldSizePayload {
                    field_name: payload.field_name,
                    field_size: payload.field_size,
                }
            }))
        }
        p3_types::ErrorCode::HttpRequestTrailerSectionSize(payload) => {
            p2_types::ErrorCode::HttpRequestTrailerSectionSize(payload)
        }
        p3_types::ErrorCode::HttpRequestTrailerSize(payload) => {
            p2_types::ErrorCode::HttpRequestTrailerSize(p2_types::FieldSizePayload {
                field_name: payload.field_name,
                field_size: payload.field_size,
            })
        }
        p3_types::ErrorCode::HttpResponseIncomplete => p2_types::ErrorCode::HttpResponseIncomplete,
        p3_types::ErrorCode::HttpResponseHeaderSectionSize(payload) => {
            p2_types::ErrorCode::HttpResponseHeaderSectionSize(payload)
        }
        p3_types::ErrorCode::HttpResponseHeaderSize(payload) => {
            p2_types::ErrorCode::HttpResponseHeaderSize(p2_types::FieldSizePayload {
                field_name: payload.field_name,
                field_size: payload.field_size,
            })
        }
        p3_types::ErrorCode::HttpResponseBodySize(payload) => {
            p2_types::ErrorCode::HttpResponseBodySize(payload)
        }
        p3_types::ErrorCode::HttpResponseTrailerSectionSize(payload) => {
            p2_types::ErrorCode::HttpResponseTrailerSectionSize(payload)
        }
        p3_types::ErrorCode::HttpResponseTrailerSize(payload) => {
            p2_types::ErrorCode::HttpResponseTrailerSize(p2_types::FieldSizePayload {
                field_name: payload.field_name,
                field_size: payload.field_size,
            })
        }
        p3_types::ErrorCode::HttpResponseTransferCoding(payload) => {
            p2_types::ErrorCode::HttpResponseTransferCoding(payload)
        }
        p3_types::ErrorCode::HttpResponseContentCoding(payload) => {
            p2_types::ErrorCode::HttpResponseContentCoding(payload)
        }
        p3_types::ErrorCode::HttpResponseTimeout => p2_types::ErrorCode::HttpResponseTimeout,
        p3_types::ErrorCode::HttpUpgradeFailed => p2_types::ErrorCode::HttpUpgradeFailed,
        p3_types::ErrorCode::HttpProtocolError => p2_types::ErrorCode::HttpProtocolError,
        p3_types::ErrorCode::LoopDetected => p2_types::ErrorCode::LoopDetected,
        p3_types::ErrorCode::ConfigurationError => p2_types::ErrorCode::ConfigurationError,
        p3_types::ErrorCode::InternalError(payload) => p2_types::ErrorCode::InternalError(payload),
    }
}
