use crate::wasi;
use std::borrow::Cow;

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
