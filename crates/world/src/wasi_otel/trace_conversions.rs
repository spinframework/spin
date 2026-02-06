use crate::wasi;
use opentelemetry_sdk::trace::{SpanEvents, SpanLinks};

impl From<wasi::otel::tracing::SpanData> for opentelemetry_sdk::trace::SpanData {
    fn from(value: wasi::otel::tracing::SpanData) -> Self {
        let mut span_events = SpanEvents::default();
        span_events.events = value.events.into_iter().map(Into::into).collect();
        span_events.dropped_count = value.dropped_events;
        let mut span_links = SpanLinks::default();
        span_links.links = value.links.into_iter().map(Into::into).collect();
        span_links.dropped_count = value.dropped_links;
        Self {
            span_context: value.span_context.into(),
            parent_span_id: opentelemetry::trace::SpanId::from_hex(&value.parent_span_id)
                .unwrap_or(opentelemetry::trace::SpanId::INVALID),
            span_kind: value.span_kind.into(),
            name: value.name.into(),
            start_time: value.start_time.into(),
            end_time: value.end_time.into(),
            attributes: value.attributes.into_iter().map(Into::into).collect(),
            dropped_attributes_count: value.dropped_attributes,
            events: span_events,
            links: span_links,
            status: value.status.into(),
            instrumentation_scope: value.instrumentation_scope.into(),
        }
    }
}

impl From<wasi::otel::tracing::SpanContext> for opentelemetry::trace::SpanContext {
    fn from(sc: wasi::otel::tracing::SpanContext) -> Self {
        let trace_id = opentelemetry::trace::TraceId::from_hex(&sc.trace_id)
            .unwrap_or(opentelemetry::trace::TraceId::INVALID);
        let span_id = opentelemetry::trace::SpanId::from_hex(&sc.span_id)
            .unwrap_or(opentelemetry::trace::SpanId::INVALID);
        let trace_state = opentelemetry::trace::TraceState::from_key_value(sc.trace_state)
            .unwrap_or_else(|_| opentelemetry::trace::TraceState::default());
        Self::new(
            trace_id,
            span_id,
            sc.trace_flags.into(),
            sc.is_remote,
            trace_state,
        )
    }
}

impl From<opentelemetry::trace::SpanContext> for wasi::otel::tracing::SpanContext {
    fn from(sc: opentelemetry::trace::SpanContext) -> Self {
        Self {
            trace_id: format!("{:x}", sc.trace_id()),
            span_id: format!("{:x}", sc.span_id()),
            trace_flags: sc.trace_flags().into(),
            is_remote: sc.is_remote(),
            trace_state: sc
                .trace_state()
                .header()
                .split(',')
                .filter_map(|s| {
                    if let Some((key, value)) = s.split_once('=') {
                        Some((key.to_string(), value.to_string()))
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
}

impl From<wasi::otel::tracing::TraceFlags> for opentelemetry::trace::TraceFlags {
    fn from(flags: wasi::otel::tracing::TraceFlags) -> Self {
        Self::new(flags.as_array()[0] as u8)
    }
}

impl From<opentelemetry::trace::TraceFlags> for wasi::otel::tracing::TraceFlags {
    fn from(flags: opentelemetry::trace::TraceFlags) -> Self {
        if flags.is_sampled() {
            wasi::otel::tracing::TraceFlags::SAMPLED
        } else {
            wasi::otel::tracing::TraceFlags::empty()
        }
    }
}

impl From<wasi::otel::tracing::SpanKind> for opentelemetry::trace::SpanKind {
    fn from(kind: wasi::otel::tracing::SpanKind) -> Self {
        match kind {
            wasi::otel::tracing::SpanKind::Client => opentelemetry::trace::SpanKind::Client,
            wasi::otel::tracing::SpanKind::Server => opentelemetry::trace::SpanKind::Server,
            wasi::otel::tracing::SpanKind::Producer => opentelemetry::trace::SpanKind::Producer,
            wasi::otel::tracing::SpanKind::Consumer => opentelemetry::trace::SpanKind::Consumer,
            wasi::otel::tracing::SpanKind::Internal => opentelemetry::trace::SpanKind::Internal,
        }
    }
}

impl From<wasi::otel::tracing::Event> for opentelemetry::trace::Event {
    fn from(event: wasi::otel::tracing::Event) -> Self {
        Self::new(
            event.name,
            event.time.into(),
            event.attributes.into_iter().map(Into::into).collect(),
            0,
        )
    }
}

impl From<wasi::otel::tracing::Link> for opentelemetry::trace::Link {
    fn from(link: wasi::otel::tracing::Link) -> Self {
        Self::new(
            link.span_context.into(),
            link.attributes.into_iter().map(Into::into).collect(),
            0,
        )
    }
}

impl From<wasi::otel::tracing::Status> for opentelemetry::trace::Status {
    fn from(status: wasi::otel::tracing::Status) -> Self {
        match status {
            wasi::otel::tracing::Status::Unset => Self::Unset,
            wasi::otel::tracing::Status::Ok => Self::Ok,
            wasi::otel::tracing::Status::Error(s) => Self::Error {
                description: s.into(),
            },
        }
    }
}

mod test {
    #[test]
    fn trace_flags() {
        let flags = opentelemetry::trace::TraceFlags::SAMPLED;
        let flags2 = crate::wasi::otel::tracing::TraceFlags::from(flags);
        let flags3 = opentelemetry::trace::TraceFlags::from(flags2);
        assert_eq!(flags, flags3);
    }

    #[test]
    fn span_context() {
        let sc = opentelemetry::trace::SpanContext::new(
            opentelemetry::trace::TraceId::from_hex("4fb34cb4484029f7881399b149e41e98").unwrap(),
            opentelemetry::trace::SpanId::from_hex("9ffd58d3cd4dd90b").unwrap(),
            opentelemetry::trace::TraceFlags::SAMPLED,
            false,
            opentelemetry::trace::TraceState::from_key_value(vec![("foo", "bar"), ("baz", "qux")])
                .unwrap(),
        );
        let sc2 = crate::wasi::otel::tracing::SpanContext::from(sc.clone());
        let sc3 = opentelemetry::trace::SpanContext::from(sc2);
        assert_eq!(sc, sc3);
    }
}
