use std::{error::Error, net::IpAddr, sync::Arc};

use anyhow::Context;
use http::{header::HOST, Request};
use http_body_util::BodyExt;
use ip_network::IpNetwork;
use rustls::ClientConfig;
use spin_factor_outbound_networking::{ComponentTlsConfigs, OutboundAllowedHosts};
use spin_factors::{wasmtime::component::ResourceTable, RuntimeFactorsInstanceState};
use tokio::{net::TcpStream, time::timeout};
use tracing::{field::Empty, instrument, Instrument};
use wasmtime_wasi::{IoImpl, IoView};
use wasmtime_wasi_http::{
    bindings::http::types::ErrorCode,
    body::HyperOutgoingBody,
    io::TokioIo,
    types::{HostFutureIncomingResponse, IncomingResponse},
    WasiHttpCtx, WasiHttpImpl, WasiHttpView,
};

use crate::{
    intercept::{InterceptOutcome, OutboundHttpInterceptor},
    wasi_2023_10_18, wasi_2023_11_10, InstanceState, OutboundHttpFactor, SelfRequestOrigin,
};

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
    wasmtime_wasi_http::bindings::http::outgoing_handler::add_to_linker_get_host(linker, get_http)?;
    wasmtime_wasi_http::bindings::http::types::add_to_linker_get_host(linker, get_http)?;

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
                    self.state.allow_private_ips,
                )
                .in_current_span(),
            ),
        ))
    }
}

async fn send_request_impl(
    mut request: Request<wasmtime_wasi_http::body::HyperOutgoingBody>,
    mut config: wasmtime_wasi_http::types::OutgoingRequestConfig,
    outbound_allowed_hosts: OutboundAllowedHosts,
    component_tls_configs: ComponentTlsConfigs,
    request_interceptor: Option<Arc<dyn OutboundHttpInterceptor>>,
    self_request_origin: Option<SelfRequestOrigin>,
    allow_private_ips: bool,
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

    Ok(send_request_handler(request, config, tls_client_config, allow_private_ips).await)
}

/// This is a fork of wasmtime_wasi_http::default_send_request_handler function
/// forked from bytecodealliance/wasmtime commit-sha 29a76b68200fcfa69c8fb18ce6c850754279a05b
/// This fork provides the ability to configure client cert auth for mTLS
async fn send_request_handler(
    mut request: http::Request<HyperOutgoingBody>,
    wasmtime_wasi_http::types::OutgoingRequestConfig {
        use_tls,
        connect_timeout,
        first_byte_timeout,
        between_bytes_timeout,
    }: wasmtime_wasi_http::types::OutgoingRequestConfig,
    tls_client_config: Arc<ClientConfig>,
    allow_private_ips: bool,
) -> Result<wasmtime_wasi_http::types::IncomingResponse, ErrorCode> {
    let authority_str = if let Some(authority) = request.uri().authority() {
        if authority.port().is_some() {
            authority.to_string()
        } else {
            let port = if use_tls { 443 } else { 80 };
            format!("{}:{port}", authority)
        }
    } else {
        return Err(ErrorCode::HttpRequestUriInvalid);
    };

    // Resolve the authority to IP addresses
    let mut socket_addrs = tokio::net::lookup_host(&authority_str)
        .await
        .map_err(|_| dns_error("address not available".into(), 0))?
        .collect::<Vec<_>>();

    // Potentially filter out private IPs
    if !allow_private_ips && !socket_addrs.is_empty() {
        socket_addrs.retain(|addr| !is_private_ip(addr.ip()));
        if socket_addrs.is_empty() {
            return Err(ErrorCode::DestinationIpProhibited);
        }
    }

    let tcp_stream = timeout(connect_timeout, TcpStream::connect(socket_addrs.as_slice()))
        .await
        .map_err(|_| ErrorCode::ConnectionTimeout)?
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::AddrNotAvailable => dns_error("address not available".into(), 0),
            _ => ErrorCode::ConnectionRefused,
        })?;

    let (mut sender, worker) = if use_tls {
        #[cfg(any(target_arch = "riscv64", target_arch = "s390x"))]
        {
            return Err(ErrorCode::InternalError(Some(
                "unsupported architecture for SSL".to_string(),
            )));
        }

        #[cfg(not(any(target_arch = "riscv64", target_arch = "s390x")))]
        {
            use rustls::pki_types::ServerName;
            let connector = tokio_rustls::TlsConnector::from(tls_client_config);
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
            let stream = TokioIo::new(stream);

            let (sender, conn) = timeout(
                connect_timeout,
                hyper::client::conn::http1::handshake(stream),
            )
            .await
            .map_err(|_| ErrorCode::ConnectionTimeout)?
            .map_err(hyper_request_error)?;

            let worker = wasmtime_wasi::runtime::spawn(async move {
                match conn.await {
                    Ok(()) => {}
                    // TODO: shouldn't throw away this error and ideally should
                    // surface somewhere.
                    Err(e) => tracing::warn!("dropping error {e}"),
                }
            });

            (sender, worker)
        }
    } else {
        let tcp_stream = TokioIo::new(tcp_stream);
        let (sender, conn) = timeout(
            connect_timeout,
            // TODO: we should plumb the builder through the http context, and use it here
            hyper::client::conn::http1::handshake(tcp_stream),
        )
        .await
        .map_err(|_| ErrorCode::ConnectionTimeout)?
        .map_err(hyper_request_error)?;

        let worker = wasmtime_wasi::runtime::spawn(async move {
            match conn.await {
                Ok(()) => {}
                // TODO: same as above, shouldn't throw this error away.
                Err(e) => tracing::warn!("dropping error {e}"),
            }
        });

        (sender, worker)
    };

    // at this point, the request contains the scheme and the authority, but
    // the http packet should only include those if addressing a proxy, so
    // remove them here, since SendRequest::send_request does not do it for us
    *request.uri_mut() = http::Uri::builder()
        .path_and_query(
            request
                .uri()
                .path_and_query()
                .map(|p| p.as_str())
                .unwrap_or("/"),
        )
        .build()
        .expect("comes from valid request");

    let resp = timeout(first_byte_timeout, sender.send_request(request))
        .await
        .map_err(|_| ErrorCode::ConnectionReadTimeout)?
        .map_err(hyper_request_error)?
        .map(|body| body.map_err(hyper_request_error).boxed());

    tracing::Span::current().record("http.response.status_code", resp.status().as_u16());

    Ok(wasmtime_wasi_http::types::IncomingResponse {
        resp,
        worker: Some(worker),
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

fn dns_error(rcode: String, info_code: u16) -> ErrorCode {
    ErrorCode::DnsError(wasmtime_wasi_http::bindings::http::types::DnsErrorPayload {
        rcode: Some(rcode),
        info_code: Some(info_code),
    })
}

/// Returns true if the IP is a private IP address.
fn is_private_ip(ip: IpAddr) -> bool {
    !IpNetwork::from(ip).is_global()
}
