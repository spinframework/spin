use opentelemetry::{
    global,
    propagation::{Extractor, Injector},
};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::env::OtelExporterOtlpHeaders;

/// Injects the current W3C TraceContext into the provided request.
pub fn inject_trace_context<'a>(req: impl Into<HeaderInjector<'a>>) {
    let mut injector = req.into();
    for (key, value) in OtelExporterOtlpHeaders::headers() {
        injector.set(&key, value);
    }

    global::get_text_map_propagator(|propagator| {
        let context = tracing::Span::current().context();
        propagator.inject_context(&context, &mut injector);
    });
}

/// Extracts the W3C TraceContext from the provided request and sets it as the parent of the
/// current span.
pub fn extract_trace_context<'a>(req: impl Into<HeaderExtractor<'a>>) {
    let extractor = req.into();
    let parent_context =
        global::get_text_map_propagator(|propagator| propagator.extract(&extractor));
    tracing::Span::current().set_parent(parent_context);
}

pub enum HeaderInjector<'a> {
    Http0(&'a mut http0::HeaderMap),
    Http1(&'a mut http1::HeaderMap),
}

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        match self {
            HeaderInjector::Http0(headers) => {
                if let Ok(name) = http0::header::HeaderName::from_bytes(key.as_bytes()) {
                    if let Ok(val) = http0::header::HeaderValue::from_str(&value) {
                        headers.insert(name, val);
                    }
                }
            }
            HeaderInjector::Http1(headers) => {
                if let Ok(name) = http1::header::HeaderName::from_bytes(key.as_bytes()) {
                    if let Ok(val) = http1::header::HeaderValue::from_str(&value) {
                        headers.insert(name, val);
                    }
                }
            }
        }
    }
}

impl<'a, T> From<&'a mut http0::Request<T>> for HeaderInjector<'a> {
    fn from(req: &'a mut http0::Request<T>) -> Self {
        Self::Http0(req.headers_mut())
    }
}

impl<'a, T> From<&'a mut http1::Request<T>> for HeaderInjector<'a> {
    fn from(req: &'a mut http1::Request<T>) -> Self {
        Self::Http1(req.headers_mut())
    }
}

impl<'a> From<&'a mut http0::HeaderMap> for HeaderInjector<'a> {
    fn from(headers: &'a mut http0::HeaderMap) -> Self {
        Self::Http0(headers)
    }
}

impl<'a> From<&'a mut http1::HeaderMap> for HeaderInjector<'a> {
    fn from(headers: &'a mut http1::HeaderMap) -> Self {
        Self::Http1(headers)
    }
}

pub enum HeaderExtractor<'a> {
    Http0(&'a http0::HeaderMap),
    Http1(&'a http1::HeaderMap),
}

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        match self {
            HeaderExtractor::Http0(headers) => {
                headers.get(key).map(|v| v.to_str().unwrap_or_default())
            }
            HeaderExtractor::Http1(headers) => {
                headers.get(key).map(|v| v.to_str().unwrap_or_default())
            }
        }
    }

    fn keys(&self) -> Vec<&str> {
        match self {
            HeaderExtractor::Http0(headers) => headers.keys().map(|k| k.as_str()).collect(),
            HeaderExtractor::Http1(headers) => headers.keys().map(|k| k.as_str()).collect(),
        }
    }
}

impl<'a, T> From<&'a http0::Request<T>> for HeaderExtractor<'a> {
    fn from(req: &'a http0::Request<T>) -> Self {
        Self::Http0(req.headers())
    }
}

impl<'a, T> From<&'a http1::Request<T>> for HeaderExtractor<'a> {
    fn from(req: &'a http1::Request<T>) -> Self {
        Self::Http1(req.headers())
    }
}
