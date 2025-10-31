use std::sync::Arc;

use http_body_util::BodyExt;
use spin_factor_outbound_networking::config::blocked_networks::BlockedNetworks;
use spin_world::v1::{
    http as spin_http,
    http_types::{self, HttpError, Method, Request, Response},
};
use tracing::{field::Empty, instrument, Span};

use crate::intercept::InterceptOutcome;

impl spin_http::Host for crate::InstanceState {
    #[instrument(name = "spin_outbound_http.send_request", skip_all,
        fields(otel.kind = "client", url.full = Empty, http.request.method = Empty,
        http.response.status_code = Empty, otel.name = Empty, server.address = Empty, server.port = Empty))]
    async fn send_request(&mut self, req: Request) -> Result<Response, HttpError> {
        let span = Span::current();
        record_request_fields(&span, &req);

        let uri = req.uri;
        tracing::trace!("Sending outbound HTTP to {uri:?}");

        if !req.params.is_empty() {
            tracing::warn!("HTTP params field is deprecated");
        }
        let req_url = if !uri.starts_with('/') {
            // Absolute URI
            let is_allowed = self
                .allowed_hosts
                .check_url(&uri, "https")
                .await
                .unwrap_or(false);
            if !is_allowed {
                return Err(HttpError::DestinationNotAllowed);
            }
            uri.parse().map_err(|_| HttpError::InvalidUrl)?
        } else {
            // Relative URI ("self" request)
            let is_allowed = self
                .allowed_hosts
                .check_relative_url(&["http", "https"])
                .await
                .unwrap_or(false);
            if !is_allowed {
                return Err(HttpError::DestinationNotAllowed);
            }

            let Some(origin) = &self.self_request_origin else {
                tracing::error!(
                    "Couldn't handle outbound HTTP request to relative URI; no origin set"
                );
                return Err(HttpError::InvalidUrl);
            };
            let path_and_query = uri.parse().map_err(|_| HttpError::InvalidUrl)?;
            origin.clone().into_uri(Some(path_and_query))
        };

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

        if let Some(interceptor) = &self.request_interceptor {
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

        // Allow reuse of Client's internal connection pool for multiple requests
        // in a single component execution
        let client = self.spin_http_client.get_or_insert_with(|| {
            let mut builder = reqwest::Client::builder()
                .dns_resolver(Arc::new(SpinDnsResolver(self.blocked_networks.clone())));
            if !self.connection_pooling_enabled {
                builder = builder.pool_max_idle_per_host(0);
            }
            builder.build().unwrap()
        });

        // If we're limiting concurrent outbound requests, acquire a permit
        // Note: since we don't have access to the underlying connection, we can only
        // limit the number of concurrent requests, not connections.
        let permit = crate::concurrent_outbound_connections::acquire_semaphore(
            "spin",
            &self.concurrent_outbound_connections_semaphore,
        )
        .await;
        let resp = client.execute(req).await.map_err(log_reqwest_error)?;
        drop(permit);

        tracing::trace!("Returning response from outbound request to {req_url}");
        span.record("http.response.status_code", resp.status().as_u16());
        response_from_reqwest(resp).await
    }
}

/// Resolves DNS using Tokio's resolver, filtering out blocked IPs.
struct SpinDnsResolver(BlockedNetworks);

impl reqwest::dns::Resolve for SpinDnsResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let blocked_networks = self.0.clone();
        Box::pin(async move {
            let mut addrs = tokio::net::lookup_host(name.as_str())
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
        .record("http.request.method", method)
        .record("url.full", req.uri.clone());
    if let Ok(uri) = req.uri.parse::<http::Uri>() {
        if let Some(authority) = uri.authority() {
            span.record("server.address", authority.host());
            if let Some(port) = authority.port() {
                span.record("server.port", port.as_u16());
            }
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

async fn response_from_hyper(mut resp: crate::Response) -> Result<Response, HttpError> {
    let status = resp.status().as_u16();

    let headers = headers_from_map(resp.headers());

    let body = resp
        .body_mut()
        .collect()
        .await
        .map_err(|_| HttpError::RuntimeError)?
        .to_bytes()
        .to_vec();

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

    let body = res
        .bytes()
        .await
        .map_err(|_| HttpError::RuntimeError)?
        .to_vec();

    Ok(Response {
        status,
        headers: Some(headers),
        body: Some(body),
    })
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
