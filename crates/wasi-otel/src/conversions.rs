use super::*;
use opentelemetry::StringValue;
use opentelemetry_sdk::trace::{SpanEvents, SpanLinks};
use std::borrow::Cow;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wasi::clocks0_2_0::wall_clock;

impl From<wasi::otel::metrics::ResourceMetrics>
    for opentelemetry_sdk::metrics::data::ResourceMetrics
{
    fn from(value: wasi::otel::metrics::ResourceMetrics) -> Self {
        Self {
            resource: value.resource.into(),
            scope_metrics: value.scope_metrics.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<wasi::otel::metrics::Resource> for opentelemetry_sdk::Resource {
    fn from(value: wasi::otel::metrics::Resource) -> Self {
        let attributes: Vec<opentelemetry::KeyValue> =
            value.attributes.into_iter().map(Into::into).collect();
        let schema_url: Option<String> = value.schema_url;
        match schema_url {
            Some(url) => opentelemetry_sdk::resource::Resource::builder()
                .with_schema_url(attributes, url)
                .build(),
            None => opentelemetry_sdk::resource::Resource::builder()
                .with_attributes(attributes)
                .build(),
        }
    }
}

impl From<wasi::otel::metrics::ScopeMetrics> for opentelemetry_sdk::metrics::data::ScopeMetrics {
    fn from(value: wasi::otel::metrics::ScopeMetrics) -> Self {
        Self {
            scope: value.scope.into(),
            metrics: value.metrics.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<wasi::otel::metrics::Metric> for opentelemetry_sdk::metrics::data::Metric {
    fn from(value: wasi::otel::metrics::Metric) -> Self {
        Self {
            name: Cow::Owned(value.name),
            description: Cow::Owned(value.description),
            unit: Cow::Owned(value.unit),
            data: value.data.into(),
        }
    }
}

/// Converts a Wasi exemplar to an OTel exemplar
macro_rules! exemplars_to_otel {
    (
            $wasi_exemplar_list:expr,
            $exemplar_type:ty
        ) => {
        $wasi_exemplar_list
            .iter()
            .map(|e| {
                let span_id: [u8; 8] = e
                    .span_id
                    .as_bytes()
                    .try_into()
                    .expect("failed to parse span ID");
                let trace_id: [u8; 16] = e
                    .trace_id
                    .as_bytes()
                    .try_into()
                    .expect("failed to parse trace ID");
                opentelemetry_sdk::metrics::data::Exemplar::<$exemplar_type> {
                    filtered_attributes: e
                        .filtered_attributes
                        .to_owned()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    time: e.time.into(),
                    value: e.value.into(),
                    span_id,
                    trace_id,
                }
            })
            .collect()
    };
}

/// Converts a WASI Gauge to an OTel Gauge
macro_rules! wasi_gauge_to_otel {
    ($gauge:expr, $number_type:ty) => {
        Box::new(opentelemetry_sdk::metrics::data::Gauge {
            data_points: $gauge
                .data_points
                .iter()
                .map(|dp| opentelemetry_sdk::metrics::data::GaugeDataPoint {
                    attributes: dp.attributes.iter().map(Into::into).collect(),
                    value: dp.value.into(),
                    exemplars: exemplars_to_otel!(dp.exemplars, $number_type),
                })
                .collect(),
            start_time: match $gauge.start_time {
                Some(t) => Some(t.into()),
                None => None,
            },
            time: $gauge.time.into(),
        })
    };
}

/// Converts a WASI Sum to an OTel Sum
macro_rules! wasi_sum_to_otel {
    ($sum:expr, $number_type:ty) => {
        Box::new(opentelemetry_sdk::metrics::data::Sum {
            data_points: $sum
                .data_points
                .iter()
                .map(|dp| opentelemetry_sdk::metrics::data::SumDataPoint {
                    attributes: dp.attributes.iter().map(Into::into).collect(),
                    exemplars: exemplars_to_otel!(dp.exemplars, $number_type),
                    value: dp.value.into(),
                })
                .collect(),
            start_time: $sum.start_time.into(),
            time: $sum.time.into(),
            temporality: $sum.temporality.into(),
            is_monotonic: $sum.is_monotonic,
        })
    };
}

/// Converts a WASI Histogram to an OTel Histogram
macro_rules! wasi_histogram_to_otel {
    ($histogram:expr, $number_type:ty) => {
        Box::new(opentelemetry_sdk::metrics::data::Histogram {
            data_points: $histogram
                .data_points
                .iter()
                .map(|dp| opentelemetry_sdk::metrics::data::HistogramDataPoint {
                    attributes: dp.attributes.iter().map(Into::into).collect(),
                    bounds: dp.bounds.to_owned(),
                    bucket_counts: dp.bucket_counts.to_owned(),
                    exemplars: exemplars_to_otel!(dp.exemplars, $number_type),
                    count: dp.count,
                    max: match dp.max {
                        Some(m) => Some(m.into()),
                        None => None,
                    },
                    min: match dp.min {
                        Some(m) => Some(m.into()),
                        None => None,
                    },
                    sum: dp.sum.into(),
                })
                .collect(),
            start_time: $histogram.start_time.into(),
            time: $histogram.time.into(),
            temporality: $histogram.temporality.into(),
        })
    };
}

/// Converts a WASI ExponentialHistogram to an OTel ExponentialHistogram
macro_rules! wasi_exponential_histogram_to_otel {
    ($histogram:expr, $number_type:ty) => {
        Box::new(opentelemetry_sdk::metrics::data::ExponentialHistogram {
            data_points: $histogram
                .data_points
                .iter()
                .map(
                    |dp| opentelemetry_sdk::metrics::data::ExponentialHistogramDataPoint {
                        attributes: dp.attributes.iter().map(Into::into).collect(),
                        exemplars: exemplars_to_otel!(dp.exemplars, $number_type),
                        count: dp.count as usize,
                        max: match dp.max {
                            Some(m) => Some(m.into()),
                            None => None,
                        },
                        min: match dp.min {
                            Some(m) => Some(m.into()),
                            None => None,
                        },
                        sum: dp.sum.into(),
                        scale: dp.scale,
                        zero_count: dp.zero_count,
                        positive_bucket: dp.positive_bucket.to_owned().into(),
                        negative_bucket: dp.negative_bucket.to_owned().into(),
                        zero_threshold: dp.zero_threshold,
                    },
                )
                .collect(),
            start_time: $histogram.start_time.into(),
            time: $histogram.time.into(),
            temporality: $histogram.temporality.into(),
        })
    };
}

impl From<wasi::otel::metrics::MetricData>
    for Box<dyn opentelemetry_sdk::metrics::data::Aggregation>
{
    fn from(value: wasi::otel::metrics::MetricData) -> Self {
        match value {
            wasi::otel::metrics::MetricData::F64Sum(s) => wasi_sum_to_otel!(s, f64),
            wasi::otel::metrics::MetricData::S64Sum(s) => wasi_sum_to_otel!(s, i64),
            wasi::otel::metrics::MetricData::U64Sum(s) => wasi_sum_to_otel!(s, u64),
            wasi::otel::metrics::MetricData::F64Gauge(g) => wasi_gauge_to_otel!(g, f64),
            wasi::otel::metrics::MetricData::S64Gauge(g) => wasi_gauge_to_otel!(g, i64),
            wasi::otel::metrics::MetricData::U64Gauge(g) => wasi_gauge_to_otel!(g, u64),
            wasi::otel::metrics::MetricData::F64Histogram(h) => wasi_histogram_to_otel!(h, f64),
            wasi::otel::metrics::MetricData::S64Histogram(h) => wasi_histogram_to_otel!(h, i64),
            wasi::otel::metrics::MetricData::U64Histogram(h) => wasi_histogram_to_otel!(h, u64),
            wasi::otel::metrics::MetricData::F64ExponentialHistogram(h) => {
                wasi_exponential_histogram_to_otel!(h, f64)
            }
            wasi::otel::metrics::MetricData::S64ExponentialHistogram(h) => {
                wasi_exponential_histogram_to_otel!(h, i64)
            }
            wasi::otel::metrics::MetricData::U64ExponentialHistogram(h) => {
                wasi_exponential_histogram_to_otel!(h, u64)
            }
        }
    }
}

impl From<wasi::otel::metrics::MetricNumber> for f64 {
    fn from(value: wasi::otel::metrics::MetricNumber) -> Self {
        match value {
            wasi::otel::metrics::MetricNumber::F64(n) => n,
            _ => panic!("error converting WASI MetricNumber to f64"),
        }
    }
}

impl From<wasi::otel::metrics::MetricNumber> for u64 {
    fn from(value: wasi::otel::metrics::MetricNumber) -> Self {
        match value {
            wasi::otel::metrics::MetricNumber::U64(n) => n,
            _ => panic!("error converting WASI MetricNumber to u64"),
        }
    }
}

impl From<wasi::otel::metrics::MetricNumber> for i64 {
    fn from(value: wasi::otel::metrics::MetricNumber) -> Self {
        match value {
            wasi::otel::metrics::MetricNumber::S64(n) => n,
            _ => panic!("error converting WASI MetricNumber to i64"),
        }
    }
}

impl From<wasi::otel::metrics::ExponentialBucket>
    for opentelemetry_sdk::metrics::data::ExponentialBucket
{
    fn from(value: wasi::otel::metrics::ExponentialBucket) -> Self {
        Self {
            offset: value.offset,
            counts: value.counts,
        }
    }
}

impl From<wasi::otel::metrics::Temporality> for opentelemetry_sdk::metrics::Temporality {
    fn from(value: wasi::otel::metrics::Temporality) -> Self {
        use opentelemetry_sdk::metrics::Temporality;
        match value {
            wasi::otel::metrics::Temporality::Cumulative => Temporality::Cumulative,
            wasi::otel::metrics::Temporality::Delta => Temporality::Delta,
            wasi::otel::metrics::Temporality::LowMemory => Temporality::LowMemory,
        }
    }
}

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

impl From<wasi::otel::tracing::KeyValue> for opentelemetry::KeyValue {
    fn from(kv: wasi::otel::tracing::KeyValue) -> Self {
        opentelemetry::KeyValue::new(kv.key, kv.value)
    }
}

impl From<&wasi::otel::tracing::KeyValue> for opentelemetry::KeyValue {
    fn from(kv: &wasi::otel::tracing::KeyValue) -> Self {
        opentelemetry::KeyValue::new(kv.key.to_owned(), kv.value.to_owned())
    }
}

impl From<wasi::otel::types::Value> for opentelemetry::Value {
    fn from(value: wasi::otel::types::Value) -> Self {
        match value {
            wasi::otel::types::Value::String(v) => v.into(),
            wasi::otel::types::Value::Bool(v) => v.into(),
            wasi::otel::types::Value::F64(v) => v.into(),
            wasi::otel::types::Value::S64(v) => v.into(),
            wasi::otel::types::Value::StringArray(v) => opentelemetry::Value::Array(
                v.into_iter()
                    .map(StringValue::from)
                    .collect::<Vec<_>>()
                    .into(),
            ),
            wasi::otel::types::Value::BoolArray(v) => opentelemetry::Value::Array(v.into()),
            wasi::otel::types::Value::F64Array(v) => opentelemetry::Value::Array(v.into()),
            wasi::otel::types::Value::S64Array(v) => opentelemetry::Value::Array(v.into()),
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

impl From<wasi::otel::types::InstrumentationScope> for opentelemetry::InstrumentationScope {
    fn from(value: wasi::otel::tracing::InstrumentationScope) -> Self {
        let builder =
            Self::builder(value.name).with_attributes(value.attributes.into_iter().map(Into::into));
        match (value.version, value.schema_url) {
            (Some(version), Some(schema_url)) => builder
                .with_version(version)
                .with_schema_url(schema_url)
                .build(),
            (Some(version), None) => builder.with_version(version).build(),
            (None, Some(schema_url)) => builder.with_schema_url(schema_url).build(),
            (None, None) => builder.build(),
        }
    }
}

impl From<wall_clock::Datetime> for SystemTime {
    fn from(timestamp: wall_clock::Datetime) -> Self {
        UNIX_EPOCH
            + Duration::from_secs(timestamp.seconds)
            + Duration::from_nanos(timestamp.nanoseconds as u64)
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
