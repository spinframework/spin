use anyhow::{bail, Result};
use opentelemetry::global;
use opentelemetry_sdk::{
    metrics::{periodic_reader_with_async_runtime::PeriodicReader, SdkMeterProvider},
    resource::{EnvResourceDetector, ResourceDetector, TelemetryResourceDetector},
    runtime::Tokio,
    Resource,
};
use tracing::Subscriber;
use tracing_opentelemetry::MetricsLayer;
use tracing_subscriber::{registry::LookupSpan, Layer};

use crate::{detector::SpinResourceDetector, env::OtlpProtocol};

/// Constructs a layer for the tracing subscriber that sends metrics to an OTEL collector.
///
/// It pulls OTEL configuration from the environment based on the variables defined
/// [here](https://opentelemetry.io/docs/specs/otel/protocol/exporter/) and
/// [here](https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#general-sdk-configuration).
pub(crate) fn otel_metrics_layer<S: Subscriber + for<'span> LookupSpan<'span>>(
    spin_version: String,
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
            .build()?,
        OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
    };

    let reader = PeriodicReader::builder(exporter, Tokio).build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();

    global::set_meter_provider(meter_provider.clone());

    Ok(MetricsLayer::new(meter_provider))
}

#[macro_export]
/// Records an increment to the named counter with the given attributes.
///
/// The increment may only be an i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of counter support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// ```no_run
/// # use spin_telemetry::metrics::counter;
/// counter!(spin.metric_name = 1, metric_attribute = "value");
/// ```
macro_rules! counter {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::trace!(counter.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

#[macro_export]
/// Adds an additional value to the distribution of the named histogram with the given attributes.
///
/// The increment may only be an i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of histogram support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// ```no_run
/// # use spin_telemetry::metrics::histogram;
/// histogram!(spin.metric_name = 1.5, metric_attribute = "value");
/// ```
macro_rules! histogram {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::trace!(histogram.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

#[macro_export]
/// Records an increment to the named monotonic counter with the given attributes.
///
/// The increment may only be a positive i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of monotonic counter support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// ```no_run
/// # use spin_telemetry::metrics::monotonic_counter;
/// monotonic_counter!(spin.metric_name = 1, metric_attribute = "value");
/// ```
macro_rules! monotonic_counter {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::trace!(monotonic_counter.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

#[macro_export]
/// Records an increment to the named monotonic counter with the given attributes.
///
/// The increment may only be a positive i64 or f64. You must not mix types for the same metric.
///
/// Takes advantage of gauge support in [tracing-opentelemetry](https://docs.rs/tracing-opentelemetry/0.32.0/tracing_opentelemetry/struct.MetricsLayer.html).
///
/// ```no_run
/// # use spin_telemetry::metrics::gauge;
/// gauge!(spin.metric_name = 1, metric_attribute = "value");
/// ```
macro_rules! gauge {
    ($metric:ident $(. $suffixes:ident)*  = $metric_value:expr $(, $attrs:ident=$values:expr)*) => {
        tracing::trace!(gauge.$metric $(. $suffixes)* = $metric_value $(, $attrs=$values)*);
    }
}

pub use counter;
pub use gauge;
pub use histogram;
pub use monotonic_counter;
