use std::sync::Arc;

use futures::stream::TryStreamExt as _;
use http_body_util::BodyExt;
use opentelemetry_semantic_conventions::attribute as otel_attribute;
use spin_factor_outbound_networking::config::allowed_hosts::OutboundAllowedHosts;
use spin_factor_outbound_networking::config::blocked_networks::BlockedNetworks;
use spin_world::MAX_HOST_BUFFERED_BYTES;
use spin_world::v1::{
    http as spin_http,
    http_types::{self, HttpError, Method, Request, Response},
};
use tracing::{Span, field::Empty, instrument};

use crate::intercept::InterceptOutcome;

impl spin_http::Host for crate::InstanceState {
    #[instrument(name = "spin_outbound_http.send_request", skip_all,
        fields(otel.kind = "client", {otel_attribute::URL_FULL} = Empty, {otel_attribute::HTTP_REQUEST_METHOD} = Empty,
        {otel_attribute::HTTP_RESPONSE_STATUS_CODE} = Empty, otel.name = Empty, {otel_attribute::SERVER_ADDRESS} = Empty, {otel_attribute::SERVER_PORT} = Empty))]
    async fn send_request(&mut self, req: Request) -> Result<Response, HttpError> {
        self.hooks.otel.reparent_tracing_span();

        let span = Span::current();
        record_request_fields(&span, &req);

        let uri = &req.uri;
        tracing::trace!("Sending outbound HTTP to {uri:?}");

        if !req.params.is_empty() {
            tracing::warn!("HTTP params field is deprecated");
        }

        let req_url = self.actual_request_url(&req).await?;

        // Build an http::Request for OutboundHttpInterceptor
        let mut req = {
            let mut builder = http::Request::builder()
                .method(hyper_method(req.method))
                .uri(&req_url);
            for (key, val) in req.headers {
                builder = builder.header(key, val);
            }
            builder.body(req.body.unwrap_or_default())
        }
        .map_err(|err| {
            tracing::error!("Error building outbound request: {err}");
            HttpError::RuntimeError
        })?;

        spin_telemetry::inject_trace_context(req.headers_mut());

        if let Some(interceptor) = &self.hooks.request_interceptor {
            let intercepted_request = std::mem::take(&mut req).into();
            match interceptor.intercept(intercepted_request).await {
                Ok(InterceptOutcome::Continue(intercepted_request)) => {
                    req = intercepted_request.into_vec_request().unwrap();
                }
                Ok(InterceptOutcome::Complete(resp)) => return response_from_hyper(resp).await,
                Err(err) => {
                    tracing::error!("Error in outbound HTTP interceptor: {err}");
                    return Err(HttpError::RuntimeError);
                }
            }
        }

        // Convert http::Request to reqwest::Request
        let req = reqwest::Request::try_from(req).map_err(|_| HttpError::InvalidUrl)?;

        // Enforce blocked networks for IP hosts
        if let Some(host) = req.url().host() {
            let ip_host: Option<std::net::IpAddr> = match host {
                url::Host::Ipv4(addr) => Some(addr.into()),
                url::Host::Ipv6(addr) => Some(addr.into()),
                url::Host::Domain(_) => None, // Handled by SpinDnsResolver
            };
            if ip_host.is_some_and(|ip| self.hooks.blocked_networks.is_blocked(&ip)) {
                tracing::error!(%host, "Connection to disallowed IP host has been blocked");
                return Err(HttpError::DestinationNotAllowed);
            }
        }

        // Allow reuse of Client's internal connection pool for multiple requests
        // in a single component execution
        let client = self.hooks.spin_http_client.get_or_insert_with(|| {
            let mut builder = reqwest::Client::builder().dns_resolver(Arc::new(SpinDnsResolver(
                self.hooks.blocked_networks.clone(),
            )));
            if !self.hooks.connection_pooling_enabled {
                builder = builder.pool_max_idle_per_host(0);
            }
            builder.build().unwrap()
        });

        // If we're limiting concurrent outbound requests, acquire a permit
        // Note: since we don't have access to the underlying connection, we can only
        // limit the number of concurrent requests, not connections.
        let permit = self
            .hooks
            .semaphore
            .acquire()
            .await
            .map_err(|_| HttpError::TooManyRequests)?;
        let resp = client.execute(req).await.map_err(log_reqwest_error)?;
        drop(permit);

        tracing::trace!("Returning response from outbound request to {req_url}");
        span.record(
            otel_attribute::HTTP_RESPONSE_STATUS_CODE,
            resp.status().as_u16(),
        );
        response_from_reqwest(resp).await
    }
}

impl crate::InstanceState {
    async fn actual_request_url(&self, req: &Request) -> Result<http::Uri, HttpError> {
        let uri = &req.uri;
        let allowed_hosts = &self.hooks.allowed_hosts;

        if !uri.starts_with('/') {
            // Absolute URI
            if !Self::is_allowed_absolute(allowed_hosts, req).await {
                return Err(HttpError::DestinationNotAllowed);
            }
            uri.parse().map_err(|_| HttpError::InvalidUrl)
        } else {
            // Relative URI ("self" request)
            if !Self::is_allowed_relative(allowed_hosts, req).await {
                return Err(HttpError::DestinationNotAllowed);
            }

            let Some(origin) = &self.hooks.self_request_origin else {
                tracing::error!(
                    "Couldn't handle outbound HTTP request to relative URI; no origin set"
                );
                return Err(HttpError::InvalidUrl);
            };
            let path_and_query = uri.parse().map_err(|_| HttpError::InvalidUrl)?;
            Ok(origin.clone().into_uri(Some(path_and_query)))
        }
    }

    // These functions take `OutboundAllowedHosts` instead of `self` to make testing easier

    async fn is_allowed_absolute(allowed_hosts: &OutboundAllowedHosts, req: &Request) -> bool {
        Self::is_uri_allowed_absolute(allowed_hosts, &req.uri).await
            && Self::is_host_header_allowed(allowed_hosts, req).await
    }

    async fn is_allowed_relative(allowed_hosts: &OutboundAllowedHosts, req: &Request) -> bool {
        Self::is_url_allowed_relative(allowed_hosts).await
            && Self::is_host_header_allowed(allowed_hosts, req).await
    }

    async fn is_url_allowed_relative(allowed_hosts: &OutboundAllowedHosts) -> bool {
        allowed_hosts
            .check_relative_url(&["http", "https"])
            .await
            .unwrap_or(false)
    }

    async fn is_uri_allowed_absolute(allowed_hosts: &OutboundAllowedHosts, uri: &str) -> bool {
        allowed_hosts.check_url(uri, "https").await.unwrap_or(false)
    }

    async fn is_host_header_allowed(allowed_hosts: &OutboundAllowedHosts, req: &Request) -> bool {
        let host_header = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("Host"))
            .map(|(_, v)| v);
        match host_header {
            Some(host) => {
                if host.starts_with("/") {
                    Self::is_url_allowed_relative(allowed_hosts).await
                } else {
                    Self::is_uri_allowed_absolute(allowed_hosts, host).await
                }
            }
            None => true,
        }
    }
}

/// Resolves DNS using Tokio's resolver, filtering out blocked IPs.
struct SpinDnsResolver(BlockedNetworks);

impl reqwest::dns::Resolve for SpinDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let blocked_networks = self.0.clone();
        Box::pin(async move {
            let mut addrs = tokio::net::lookup_host((name.as_str(), 0))
                .await
                .map_err(Box::new)?
                .collect::<Vec<_>>();
            // Remove blocked IPs
            crate::remove_blocked_addrs(&blocked_networks, &mut addrs).map_err(Box::new)?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

impl http_types::Host for crate::InstanceState {
    fn convert_http_error(&mut self, err: HttpError) -> anyhow::Result<HttpError> {
        Ok(err)
    }
}

fn record_request_fields(span: &Span, req: &Request) {
    let method = match req.method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
        Method::Patch => "PATCH",
        Method::Head => "HEAD",
        Method::Options => "OPTIONS",
    };
    // Set otel.name to just the method name to fit with OpenTelemetry conventions
    // <https://opentelemetry.io/docs/specs/semconv/http/http-spans/#name>
    span.record("otel.name", method)
        .record(otel_attribute::HTTP_REQUEST_METHOD, method)
        .record(otel_attribute::URL_FULL, req.uri.clone());
    if let Ok(uri) = req.uri.parse::<http::Uri>()
        && let Some(authority) = uri.authority()
    {
        span.record(otel_attribute::SERVER_ADDRESS, authority.host());
        if let Some(port) = authority.port() {
            span.record(otel_attribute::SERVER_PORT, port.as_u16());
        }
    }
}

fn hyper_method(m: Method) -> http::Method {
    match m {
        Method::Get => http::Method::GET,
        Method::Post => http::Method::POST,
        Method::Put => http::Method::PUT,
        Method::Delete => http::Method::DELETE,
        Method::Patch => http::Method::PATCH,
        Method::Head => http::Method::HEAD,
        Method::Options => http::Method::OPTIONS,
    }
}

async fn response_from_hyper(resp: crate::Response) -> Result<Response, HttpError> {
    let status = resp.status().as_u16();

    let headers = headers_from_map(resp.headers());

    let header_bytes = std::mem::size_of::<Vec<(String, String)>>()
        + headers
            .iter()
            .map(|(k, v)| std::mem::size_of::<(String, String)>() + k.len() + v.len())
            .sum::<usize>();

    let mut stream = resp.into_body().into_data_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|_| HttpError::RuntimeError)?
    {
        body.extend(chunk);
        check_byte_count(header_bytes + body.len())?;
    }

    // One more check in case the body was empty:
    check_byte_count(header_bytes + body.len())?;

    Ok(Response {
        status,
        headers: Some(headers),
        body: Some(body),
    })
}

fn log_reqwest_error(err: reqwest::Error) -> HttpError {
    let error_desc = if err.is_timeout() {
        "timeout error"
    } else if err.is_connect() {
        "connection error"
    } else if err.is_body() || err.is_decode() {
        "message body error"
    } else if err.is_request() {
        "request error"
    } else {
        "error"
    };
    tracing::warn!(
        "Outbound HTTP {}: URL {}, error detail {:?}",
        error_desc,
        err.url()
            .map(|u| u.to_string())
            .unwrap_or_else(|| "<unknown>".to_owned()),
        err
    );
    HttpError::RuntimeError
}

async fn response_from_reqwest(res: reqwest::Response) -> Result<Response, HttpError> {
    let status = res.status().as_u16();

    let headers = headers_from_map(res.headers());

    let header_bytes = std::mem::size_of::<Vec<(String, String)>>()
        + headers
            .iter()
            .map(|(k, v)| std::mem::size_of::<(String, String)>() + k.len() + v.len())
            .sum::<usize>();

    let mut stream = res.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|_| HttpError::RuntimeError)?
    {
        body.extend(chunk);
        check_byte_count(header_bytes + body.len())?;
    }

    // One more check in case the body was empty:
    check_byte_count(header_bytes + body.len())?;

    Ok(Response {
        status,
        headers: Some(headers),
        body: Some(body),
    })
}

fn check_byte_count(count: usize) -> Result<(), HttpError> {
    if count > MAX_HOST_BUFFERED_BYTES {
        tracing::warn!("query result exceeds limit of {MAX_HOST_BUFFERED_BYTES} bytes");
        Err(HttpError::RuntimeError)
    } else {
        Ok(())
    }
}

fn headers_from_map(map: &http::HeaderMap) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(key, val)| {
            Some((
                key.to_string(),
                val.to_str()
                    .ok()
                    .or_else(|| {
                        tracing::warn!("Non-ascii response header value for {key}");
                        None
                    })?
                    .to_string(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::FutureExt;
    use spin_factor_outbound_networking::config::allowed_hosts::{
        AllowedHostConfig, AllowedHostsConfig,
    };

    fn allowed_hostify(allowed_hosts_list: &[&str]) -> OutboundAllowedHosts {
        let allowed_host_configs = allowed_hosts_list
            .iter()
            .map(|s| AllowedHostConfig::parse(*s).unwrap())
            .collect();
        let allowed_hosts_config = AllowedHostsConfig::SpecificHosts(allowed_host_configs);
        let allowed_hosts_config_fut = async move { Ok(Arc::new(allowed_hosts_config)) };
        OutboundAllowedHosts::new(allowed_hosts_config_fut.boxed().shared(), None, false)
    }

    fn headers(host_header: Option<&str>) -> Vec<(String, String)> {
        match host_header {
            None => vec![],
            Some(h) => vec![("Host".into(), h.into())],
        }
    }

    fn requestify(uri: &str, host_header: Option<&str>) -> Request {
        let headers = headers(host_header);
        Request {
            method: Method::Get,
            uri: uri.into(),
            headers,
            params: vec![],
            body: None,
        }
    }

    async fn assert_allowed_absolute(
        uri: &str,
        host_header: Option<&str>,
        allowed_hosts_list: &[&str],
    ) {
        let allowed_hosts = allowed_hostify(allowed_hosts_list);
        let req = requestify(uri, host_header);

        assert!(
            crate::InstanceState::is_allowed_absolute(&allowed_hosts, &req).await,
            "expected {uri} to validate against {allowed_hosts_list:?}"
        );
    }

    async fn assert_not_allowed_absolute(
        uri: &str,
        host_header: Option<&str>,
        allowed_hosts_list: &[&str],
    ) {
        let allowed_hosts = allowed_hostify(allowed_hosts_list);
        let req = requestify(uri, host_header);

        assert!(
            !crate::InstanceState::is_allowed_absolute(&allowed_hosts, &req).await,
            "expected {uri} to validate against {allowed_hosts_list:?}"
        );
    }

    async fn assert_allowed_relative(
        uri: &str,
        host_header: Option<&str>,
        allowed_hosts_list: &[&str],
    ) {
        let allowed_hosts = allowed_hostify(allowed_hosts_list);
        let req = requestify(uri, host_header);

        assert!(
            crate::InstanceState::is_allowed_relative(&allowed_hosts, &req).await,
            "expected {uri} to validate against {allowed_hosts_list:?}"
        );
    }

    async fn assert_not_allowed_relative(
        uri: &str,
        host_header: Option<&str>,
        allowed_hosts_list: &[&str],
    ) {
        let allowed_hosts = allowed_hostify(allowed_hosts_list);
        let req = requestify(uri, host_header);

        assert!(
            !crate::InstanceState::is_allowed_relative(&allowed_hosts, &req).await,
            "expected {uri} to validate against {allowed_hosts_list:?}"
        );
    }

    async fn assert_allowed(uri: &str, host_header: Option<&str>, allowed_hosts_list: &[&str]) {
        if uri.starts_with('/') {
            assert_allowed_relative(uri, host_header, allowed_hosts_list).await
        } else {
            assert_allowed_absolute(uri, host_header, allowed_hosts_list).await
        }
    }

    async fn assert_not_allowed(uri: &str, host_header: Option<&str>, allowed_hosts_list: &[&str]) {
        if uri.starts_with('/') {
            assert_not_allowed_relative(uri, host_header, allowed_hosts_list).await
        } else {
            assert_not_allowed_absolute(uri, host_header, allowed_hosts_list).await
        }
    }

    #[tokio::test]
    async fn validates_hosts() {
        let allowed = &["https://okay.com", "https://legit.com"];

        assert_allowed("https://okay.com/", None, allowed).await;
        assert_allowed("https://okay.com/", Some("okay.com"), allowed).await;
        assert_allowed("https://okay.com/", Some("legit.com"), allowed).await;

        assert_not_allowed("https://crimes.com/", None, allowed).await;
        assert_not_allowed("https://crimes.com/", Some("legit.com"), allowed).await;
        assert_not_allowed("https://crimes.com/", Some("evil.com"), allowed).await;

        assert_not_allowed("https://okay.com/", Some("evil.com"), allowed).await;

        let allowed = &["https://okay.com", "http://self"];

        assert_allowed("/foo", None, allowed).await;
        assert_allowed("/foo", Some("okay.com"), allowed).await;
        assert_not_allowed("/foo", Some("evil.com"), allowed).await;
    }
}
