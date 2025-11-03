use std::{ascii::escape_default, sync::OnceLock};

use anyhow::bail;
use opentelemetry::logs::{LogRecord, Logger, LoggerProvider};
use opentelemetry_sdk::{
    logs::{log_processor_with_async_runtime::BatchLogProcessor, BatchConfigBuilder, SdkLogger},
    resource::{EnvResourceDetector, ResourceDetector, TelemetryResourceDetector},
    runtime::Tokio,
    Resource,
};

use crate::{
    detector::SpinResourceDetector,
    env::{self, otel_logs_enabled, OtlpProtocol},
};

static LOGGER: OnceLock<SdkLogger> = OnceLock::new();

/// Handle an application log. Has the potential to both forward the log to OTel and to emit it as a
/// tracing event.
pub fn handle_app_log(buf: &[u8]) {
    app_log_to_otel(buf);
    app_log_to_tracing_event(buf);
}

/// Forward the app log to OTel.
fn app_log_to_otel(buf: &[u8]) {
    if !otel_logs_enabled() {
        return;
    }

    if let Some(logger) = LOGGER.get() {
        if let Ok(s) = std::str::from_utf8(buf) {
            let mut record = logger.create_log_record();
            record.set_body(s.to_string().into());
            logger.emit(record);
        } else {
            let mut record = logger.create_log_record();
            record.set_body(escape_non_utf8_buf(buf).into());
            record.add_attribute("app_log_non_utf8", true);
            logger.emit(record);
        }
    } else {
        tracing::trace!("OTel logger not initialized, failed to log");
    }
}

/// Takes a Spin application log and emits it as a tracing event. This acts as a compatibility layer
/// to easily get Spin app logs as events in our OTel traces.
fn app_log_to_tracing_event(buf: &[u8]) {
    static CELL: OnceLock<bool> = OnceLock::new();
    if *CELL.get_or_init(env::spin_disable_log_to_tracing) {
        return;
    }

    if let Ok(s) = std::str::from_utf8(buf) {
        tracing::info!(app_log = s);
    } else {
        tracing::info!(app_log_non_utf8 = escape_non_utf8_buf(buf));
    }
}

fn escape_non_utf8_buf(buf: &[u8]) -> String {
    buf.iter()
        .take(50)
        .map(|&x| escape_default(x).to_string())
        .collect::<String>()
}

/// Initialize the OTel logging backend.
pub(crate) fn init_otel_logging_backend(spin_version: String) -> anyhow::Result<()> {
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
    // combination of OTEL_EXPORTER_OTLP_PROTOCOL and OTEL_EXPORTER_OTLP_LOGS_PROTOCOL to
    // determine whether we should use http/protobuf or grpc.
    let exporter = match OtlpProtocol::logs_protocol_from_env() {
        OtlpProtocol::Grpc => opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .build()?,
        OtlpProtocol::HttpProtobuf => opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .build()?,
        OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
    };

    let provider = opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_log_processor(
            BatchLogProcessor::builder(exporter, Tokio)
                .with_batch_config(BatchConfigBuilder::default().build())
                .build(),
        )
        .build();

    let _ = LOGGER.set(provider.logger("spin"));
    Ok(())
}
