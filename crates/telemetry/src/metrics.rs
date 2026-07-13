use anyhow::{Result, bail};
use opentelemetry::global;
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
use tracing::Subscriber;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::{Layer, registry::LookupSpan};

use crate::{detector::SpinResourceDetector, env::OtlpProtocol};

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

/// Constructs a layer for the tracing subscriber that sends metrics to an OTEL collector.
///
/// It pulls OTEL configuration from the environment based on the variables defined
/// [here](https://opentelemetry.io/docs/specs/otel/protocol/exporter/) and
/// [here](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#general-sdk-configuration).
pub(crate) fn otel_metrics_layer<S: Subscriber + for<'span> LookupSpan<'span>>(
    spin_version: String,
    histogram_buckets: Vec<HistogramBuckets>,
) -> Result<impl Layer<S>> {
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
    let meter_provider = provider_builder.build();

    global::set_meter_provider(meter_provider.clone());

    Ok(MetricsLayer::new(meter_provider))
}

// The two mutually exclusive features cannot both select a default level.
#[cfg(all(
    feature = "metrics-default-level-debug",
    feature = "metrics-default-level-info"
))]
compile_error!(
    "features `metrics-default-level-debug` and `metrics-default-level-info` are mutually exclusive"
);

/// The [`tracing::Level`] at which the metrics macros (`counter!`, `histogram!`,
/// `monotonic_counter!`, `gauge!`) emit their events when no explicit `level:` is given.
///
/// Defaults to [`tracing::Level::TRACE`]. Enable the `metrics-default-level-debug` or
/// `metrics-default-level-info` feature on `spin-telemetry` to raise it to `DEBUG` or `INFO`.
#[cfg(all(
    not(feature = "metrics-default-level-debug"),
    not(feature = "metrics-default-level-info")
))]
pub const DEFAULT_METRICS_LEVEL: tracing::Level = tracing::Level::TRACE;

/// The [`tracing::Level`] at which the metrics macros (`counter!`, `histogram!`,
/// `monotonic_counter!`, `gauge!`) emit their events when no explicit `level:` is given.
///
/// Defaults to [`tracing::Level::DEBUG`]. Enable the `metrics-default-level-info` or
/// remove the `metrics-default-level-debug` feature on `spin-telemetry` to raise it to `INFO`
/// or lower it to `TRACE` respectively.
#[cfg(all(
    feature = "metrics-default-level-debug",
    not(feature = "metrics-default-level-info")
))]
pub const DEFAULT_METRICS_LEVEL: tracing::Level = tracing::Level::DEBUG;

/// The [`tracing::Level`] at which the metrics macros (`counter!`, `histogram!`,
/// `monotonic_counter!`, `gauge!`) emit their events when no explicit `level:` is given.
///
/// Defaults to [`tracing::Level::INFO`]. Enable the `metrics-default-level-debug` or
/// remove the `metrics-default-level-info` feature on `spin-telemetry` to lower it to `DEBUG`
/// or `TRACE` respectively.
#[cfg(all(
    feature = "metrics-default-level-info",
    not(feature = "metrics-default-level-debug")
))]
pub const DEFAULT_METRICS_LEVEL: tracing::Level = tracing::Level::INFO;

#[macro_export]
/// Records an increment to the named counter with the given attributes.
///
/// The increment may only be an i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of counter support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// The metric event is emitted at [`DEFAULT_METRICS_LEVEL`] (`TRACE` unless raised by a crate feature).
///
/// ```no_run
/// # use spin_telemetry::metrics::counter;
/// counter!(spin.metric_name = 1, metric_attribute = "value");
/// ```
macro_rules! counter {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::event!($crate::metrics::DEFAULT_METRICS_LEVEL, counter.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

#[macro_export]
/// Adds an additional value to the distribution of the named histogram with the given attributes.
///
/// The increment may only be an i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of histogram support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// The metric event is emitted at [`DEFAULT_METRICS_LEVEL`] (`TRACE` unless raised by a crate feature).
///
/// ```no_run
/// # use spin_telemetry::metrics::histogram;
/// histogram!(spin.metric_name = 1.5, metric_attribute = "value");
/// ```
macro_rules! histogram {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::event!($crate::metrics::DEFAULT_METRICS_LEVEL, histogram.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

#[macro_export]
/// Records an increment to the named monotonic counter with the given attributes.
///
/// The increment may only be a positive i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of monotonic counter support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// The metric event is emitted at [`DEFAULT_METRICS_LEVEL`] (`TRACE` unless raised by a crate feature).
///
/// ```no_run
/// # use spin_telemetry::metrics::monotonic_counter;
/// monotonic_counter!(spin.metric_name = 1, metric_attribute = "value");
/// ```
macro_rules! monotonic_counter {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::event!($crate::metrics::DEFAULT_METRICS_LEVEL, monotonic_counter.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

#[macro_export]
/// Records the current value of the named gauge with the given attributes.
///
/// The value may only be a positive i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of gauge support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// The metric event is emitted at [`DEFAULT_METRICS_LEVEL`] (`TRACE` unless raised by a crate feature).
///
/// ```no_run
/// # use spin_telemetry::metrics::gauge;
/// gauge!(spin.metric_name = 1, metric_attribute = "value");
/// ```
macro_rules! gauge {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::event!($crate::metrics::DEFAULT_METRICS_LEVEL, gauge.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

pub use counter;
pub use gauge;
pub use histogram;
pub use monotonic_counter;
