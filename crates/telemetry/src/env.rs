use std::env::VarError;

use opentelemetry_otlp::{
    OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
    OTEL_EXPORTER_OTLP_METRICS_ENDPOINT, OTEL_EXPORTER_OTLP_PROTOCOL,
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
};

const OTEL_SDK_DISABLED: &str = "OTEL_SDK_DISABLED";
const OTEL_EXPORTER_OTLP_TRACES_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_TRACES_PROTOCOL";
const OTEL_EXPORTER_OTLP_METRICS_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_METRICS_PROTOCOL";
const OTEL_EXPORTER_OTLP_LOGS_PROTOCOL: &str = "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL";
const SPIN_DISABLE_LOG_TO_TRACING: &str = "SPIN_DISABLE_LOG_TO_TRACING";

/// Returns a boolean indicating if the OTEL tracing layer should be enabled.
///
/// It is considered enabled if any of the following environment variables are set and not empty:
/// - `OTEL_EXPORTER_OTLP_ENDPOINT`
/// - `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`
///
/// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
pub fn otel_tracing_enabled() -> bool {
    any_vars_set(&[
        OTEL_EXPORTER_OTLP_ENDPOINT,
        OTEL_EXPORTER_OTLP_TRACES_ENDPOINT,
    ]) && !otel_sdk_disabled()
}

/// Returns a boolean indicating if the OTEL metrics layer should be enabled.
///
/// It is considered enabled if any of the following environment variables are set and not empty:
/// - `OTEL_EXPORTER_OTLP_ENDPOINT`
/// - `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`
///
/// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
pub fn otel_metrics_enabled() -> bool {
    any_vars_set(&[
        OTEL_EXPORTER_OTLP_ENDPOINT,
        OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
    ]) && !otel_sdk_disabled()
}

/// Returns a boolean indicating if the OTEL log layer should be enabled.
///
/// It is considered enabled if any of the following environment variables are set and not empty:
/// - `OTEL_EXPORTER_OTLP_ENDPOINT`
/// - `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`
///
/// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
pub fn otel_logs_enabled() -> bool {
    any_vars_set(&[
        OTEL_EXPORTER_OTLP_ENDPOINT,
        OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
    ]) && !otel_sdk_disabled()
}

/// Returns a boolean indicating if the compatibility layer that emits tracing events from
/// applications logs should be disabled.
///
/// It is considered disabled if the environment variable `SPIN_DISABLED_LOG_TO_TRACING` is set and not
/// empty. By default the features is enabled.
pub fn spin_disable_log_to_tracing() -> bool {
    any_vars_set(&[SPIN_DISABLE_LOG_TO_TRACING])
}

fn any_vars_set(enabling_vars: &[&str]) -> bool {
    enabling_vars
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|val| !val.is_empty()))
}

/// Returns a boolean indicating if the OTEL SDK should be disabled for all signals.
///
/// It is considered disabled if the environment variable `OTEL_SDK_DISABLED` is set and not empty.
pub fn otel_sdk_disabled() -> bool {
    std::env::var_os(OTEL_SDK_DISABLED).is_some_and(|val| !val.is_empty())
}

/// The protocol to use for OTLP exporter.
#[derive(Debug)]
pub enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
    HttpJson,
}

impl OtlpProtocol {
    /// Returns the protocol to be used for exporting traces as defined by the environment.
    pub fn traces_protocol_from_env() -> Self {
        Self::protocol_from_env(
            std::env::var(OTEL_EXPORTER_OTLP_TRACES_PROTOCOL),
            std::env::var(OTEL_EXPORTER_OTLP_PROTOCOL),
        )
    }

    /// Returns the protocol to be used for exporting metrics as defined by the environment.
    pub fn metrics_protocol_from_env() -> Self {
        Self::protocol_from_env(
            std::env::var(OTEL_EXPORTER_OTLP_METRICS_PROTOCOL),
            std::env::var(OTEL_EXPORTER_OTLP_PROTOCOL),
        )
    }

    /// Returns the protocol to be used for exporting logs as defined by the environment.
    pub fn logs_protocol_from_env() -> Self {
        Self::protocol_from_env(
            std::env::var(OTEL_EXPORTER_OTLP_LOGS_PROTOCOL),
            std::env::var(OTEL_EXPORTER_OTLP_PROTOCOL),
        )
    }

    fn protocol_from_env(
        specific_protocol: Result<String, VarError>,
        general_protocol: Result<String, VarError>,
    ) -> Self {
        let protocol =
            specific_protocol.unwrap_or(general_protocol.unwrap_or("http/protobuf".to_string()));

        static WARN_ONCE: std::sync::Once = std::sync::Once::new();

        match protocol.as_str() {
            "grpc" => Self::Grpc,
            "http/protobuf" => Self::HttpProtobuf,
            "http/json" => Self::HttpJson,
            s => {
                WARN_ONCE.call_once(|| {
                    terminal::warn!(
                        "'{s}' is not a valid OTLP protocol, defaulting to http/protobuf"
                    );
                });
                Self::HttpProtobuf
            }
        }
    }
}
