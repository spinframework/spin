use anyhow::{Result, bail};
use opentelemetry_otlp::WithHttpConfig;
use opentelemetry_sdk::{
    Resource,
    metrics::{
        Aggregation, Instrument, SdkMeterProvider, Stream, new_view,
        periodic_reader_with_async_runtime::PeriodicReader,
    },
    resource::{EnvResourceDetector, ResourceDetector, TelemetryResourceDetector},
    runtime::Tokio,
};

use crate::{detector::SpinResourceDetector, env::OtlpProtocol};

/// Re-exported so the metric macros can refer to `$crate::opentelemetry::...`.
#[doc(hidden)]
pub use opentelemetry;

/// A custom histogram bucketing for a named metric.
///
/// OTel's default histogram boundaries are tuned for millisecond-scale durations (they top out at
/// 10000). Metrics recorded on a different scale (e.g. a 0.0..=1.0 ratio) need their own
/// boundaries, or every sample collapses into a single bucket. Callers describe such metrics with
/// this type and hand them to [`crate::init`]; this crate has no built-in knowledge of which
/// metrics need it.
pub struct HistogramBuckets {
    /// The instrument (metric) name these boundaries apply to.
    pub metric_name: &'static str,
    /// Explicit upper bounds for the histogram buckets.
    pub boundaries: Vec<f64>,
}

/// Builds an [`SdkMeterProvider`] configured to export to an OTLP collector.
///
/// It pulls OTEL configuration from the environment based on the variables defined
/// [here](https://opentelemetry.io/docs/specs/otel/protocol/exporter/) and
/// [here](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#general-sdk-configuration).
///
/// The caller is responsible for registering the returned provider as the global one (e.g. via
/// [`opentelemetry::global::set_meter_provider`]). Instruments created by the macros in this
/// module (e.g. [`monotonic_counter_u64`](crate::monotonic_counter_u64)) bind to whatever meter
/// provider is global *at the time they're first used*, and never rebind afterwards.
pub(crate) fn metrics_provider(
    spin_version: String,
    histogram_buckets: Vec<HistogramBuckets>,
) -> Result<SdkMeterProvider> {
    let resource = Resource::builder()
        .with_detectors(&[
            // Set service.name from env OTEL_SERVICE_NAME > env OTEL_RESOURCE_ATTRIBUTES > spin
            // Set service.version from Spin metadata
            Box::new(SpinResourceDetector::new(spin_version)) as Box<dyn ResourceDetector>,
            // Sets fields from env OTEL_RESOURCE_ATTRIBUTES
            Box::new(EnvResourceDetector::new()),
            // Sets telemetry.sdk{name, language, version}
            Box::new(TelemetryResourceDetector),
        ])
        .build();

    // This will configure the exporter based on the OTEL_EXPORTER_* environment variables. We
    // currently default to using the HTTP exporter but in the future we could select off of the
    // combination of OTEL_EXPORTER_OTLP_PROTOCOL and OTEL_EXPORTER_OTLP_TRACES_PROTOCOL to
    // determine whether we should use http/protobuf or grpc.
    let exporter = match OtlpProtocol::metrics_protocol_from_env() {
        OtlpProtocol::Grpc => opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .build()?,
        OtlpProtocol::HttpProtobuf => opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_http_client(crate::rustls_reqwest_client()?)
            .build()?,
        OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
    };

    let reader = PeriodicReader::builder(exporter, Tokio).build();
    let mut provider_builder = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource);
    // Apply any caller-supplied histogram bucket overrides as views. This crate stays agnostic
    // about which metrics need custom boundaries — the owning crate describes them.
    for buckets in histogram_buckets {
        provider_builder = provider_builder.with_view(new_view(
            Instrument::new().name(buckets.metric_name),
            Stream::new().aggregation(Aggregation::ExplicitBucketHistogram {
                boundaries: buckets.boundaries,
                record_min_max: true,
            }),
        )?);
    }
    Ok(provider_builder.build())
}

/// Builds a metric name from a dotted-ident path (`spin.foo.bar` => `"spin.foo.bar"`), gets or
/// creates a static instrument for it, and records a value with the given attributes.
/// Shared by every public macro in this module.
///
/// Each macro invocation expands inline at its call site, so the `static` below is a distinct
/// instrument per call site (not shared across calls).
#[doc(hidden)]
#[macro_export]
macro_rules! __otel_metric_record {
    (
        $T:ty, $builder:ident, $record_method:ident,
        $metric:ident $(. $suffixes:ident)* = $metric_value:expr $(, $attrs:ident = $values:expr)*
    ) => {{
        static INSTRUMENT: ::std::sync::LazyLock<$T> = ::std::sync::LazyLock::new(|| {
            $crate::metrics::opentelemetry::global::meter(env!("CARGO_PKG_NAME"))
                .$builder(::std::concat!(
                    ::std::stringify!($metric) $(, ".", ::std::stringify!($suffixes))*
                ))
                .build()
        });
        INSTRUMENT.$record_method(
            $metric_value,
            &[$( $crate::metrics::opentelemetry::KeyValue::new(::std::stringify!($attrs), $values) ),*],
        );
    }};
}

/// Records an increment to the named monotonic counter (as a `u64`) with the given attributes.
///
/// ```
/// spin_telemetry::metrics::monotonic_counter_u64!(spin.metric_name = 1, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! monotonic_counter_u64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Counter<u64>, u64_counter, add, $($tt)*
        )
    };
}

/// Records an increment to the named monotonic counter (as an `f64`) with the given attributes.
///
/// The increment must be non-negative.
///
/// ```
/// spin_telemetry::metrics::monotonic_counter_f64!(spin.metric_name = 1.5, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! monotonic_counter_f64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Counter<f64>, f64_counter, add, $($tt)*
        )
    };
}

/// Records a delta to the named counter (as an `i64`) with the given attributes.
///
/// Unlike `monotonic_counter_*`, the delta may be negative. This maps to OTel's `UpDownCounter`.
///
/// ```
/// spin_telemetry::metrics::counter_i64!(spin.metric_name = -1, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! counter_i64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::UpDownCounter<i64>, i64_up_down_counter, add, $($tt)*
        )
    };
}

/// Records a delta to the named counter (as an `f64`) with the given attributes.
///
/// Unlike `monotonic_counter_*`, the delta may be negative. This maps to OTel's `UpDownCounter`.
///
/// ```
/// spin_telemetry::metrics::counter_f64!(spin.metric_name = -1.5, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! counter_f64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::UpDownCounter<f64>, f64_up_down_counter, add, $($tt)*
        )
    };
}

/// Records an additional value (as a `u64`) to the distribution of the named histogram with the
/// given attributes.
///
/// ```
/// spin_telemetry::metrics::histogram_u64!(spin.metric_name = 1, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! histogram_u64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Histogram<u64>, u64_histogram, record, $($tt)*
        )
    };
}

/// Records an additional value (as an `f64`) to the distribution of the named histogram with the
/// given attributes.
///
/// ```
/// spin_telemetry::metrics::histogram_f64!(spin.metric_name = 1.5, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! histogram_f64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Histogram<f64>, f64_histogram, record, $($tt)*
        )
    };
}

/// Records the current value (as a `u64`) of the named gauge with the given attributes.
///
/// ```
/// spin_telemetry::metrics::gauge_u64!(spin.metric_name = 1, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! gauge_u64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Gauge<u64>, u64_gauge, record, $($tt)*
        )
    };
}

/// Records the current value (as an `i64`) of the named gauge with the given attributes.
///
/// ```
/// spin_telemetry::metrics::gauge_i64!(spin.metric_name = 1, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! gauge_i64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Gauge<i64>, i64_gauge, record, $($tt)*
        )
    };
}

/// Records the current value (as an `f64`) of the named gauge with the given attributes.
///
/// ```
/// spin_telemetry::metrics::gauge_f64!(spin.metric_name = 1.5, metric_attribute = "value");
/// ```
#[macro_export]
macro_rules! gauge_f64 {
    ($($tt:tt)*) => {
        $crate::__otel_metric_record!(
            $crate::metrics::opentelemetry::metrics::Gauge<f64>, f64_gauge, record, $($tt)*
        )
    };
}

pub use counter_f64;
pub use counter_i64;
pub use gauge_f64;
pub use gauge_i64;
pub use gauge_u64;
pub use histogram_f64;
pub use histogram_u64;
pub use monotonic_counter_f64;
pub use monotonic_counter_u64;
