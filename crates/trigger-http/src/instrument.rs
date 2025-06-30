use anyhow::Result;
use http::Response;
use tracing::Level;

use crate::Body;

/// Create a span for an HTTP request.
macro_rules! http_span {
    ($request:tt, $addr:tt) => {
        tracing::info_span!(
            "spin_trigger_http.handle_http_request",
            "otel.kind" = "server",
            "http.request.method" = %$request.method(),
            "network.peer.address" = %$addr.ip(),
            "network.peer.port" = %$addr.port(),
            "network.protocol.name" = "http",
            "url.path" = $request.uri().path(),
            "url.query" = $request.uri().query().unwrap_or(""),
            "url.scheme" = $request.uri().scheme_str().unwrap_or(""),
            "client.address" = $request.headers().get("x-forwarded-for").and_then(|val| val.to_str().ok()),
            // Recorded later
            "error.type" = ::tracing::field::Empty,
            "http.response.status_code" = ::tracing::field::Empty,
            "http.route" = ::tracing::field::Empty,
            "otel.name" = ::tracing::field::Empty,
        )
    };
}

pub(crate) use http_span;

/// Finish setting attributes on the HTTP span.
pub(crate) fn finalize_http_span(
    response: Result<Response<Body>>,
    method: String,
) -> Result<Response<Body>> {
    let span = tracing::Span::current();
    match response {
        Ok(response) => {
            tracing::info!(
                "Request finished, sending response with status code {}",
                response.status()
            );

            let matched_route = response.extensions().get::<MatchedRoute>();
            // Set otel.name and http.route
            if let Some(MatchedRoute { route }) = matched_route {
                span.record("http.route", route);
                span.record("otel.name", format!("{method} {route}"));
            } else {
                span.record("otel.name", method);
            }

            // Set status code
            span.record("http.response.status_code", response.status().as_u16());

            Ok(response)
        }
        Err(err) => {
            instrument_error(&err);
            span.record("http.response.status_code", 500);
            span.record("otel.name", method);
            Err(err)
        }
    }
}

/// Marks the current span as errored.
pub(crate) fn instrument_error(err: &anyhow::Error) {
    let span = tracing::Span::current();
    tracing::event!(target:module_path!(), Level::INFO, error = %err);
    span.record("error.type", format!("{err:?}"));
}

/// MatchedRoute is used as a response extension to track the route that was matched for OTel
/// tracing purposes.
#[derive(Clone)]
pub struct MatchedRoute {
    pub route: String,
}

impl MatchedRoute {
    pub fn set_response_extension(resp: &mut Response<Body>, route: impl Into<String>) {
        resp.extensions_mut().insert(MatchedRoute {
            route: route.into(),
        });
    }

    pub fn with_response_extension(
        mut resp: Response<Body>,
        route: impl Into<String>,
    ) -> Response<Body> {
        Self::set_response_extension(&mut resp, route);
        resp
    }
}
