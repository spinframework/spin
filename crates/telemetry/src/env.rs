use std::{collections::HashMap, env::VarError};

use opentelemetry_otlp::{
    OTEL_EXPORTER_OTLP_ENDPOINT, OTEL_EXPORTER_OTLP_HEADERS, OTEL_EXPORTER_OTLP_LOGS_ENDPOINT,
    OTEL_EXPORTER_OTLP_LOGS_HEADERS, OTEL_EXPORTER_OTLP_METRICS_ENDPOINT,
    OTEL_EXPORTER_OTLP_METRICS_HEADERS, OTEL_EXPORTER_OTLP_PROTOCOL,
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT, OTEL_EXPORTER_OTLP_TRACES_HEADERS,
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
pub(crate) fn otel_tracing_enabled() -> bool {
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
pub(crate) fn otel_metrics_enabled() -> bool {
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
pub(crate) fn otel_logs_enabled() -> bool {
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
pub(crate) fn spin_disable_log_to_tracing() -> bool {
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
pub(crate) fn otel_sdk_disabled() -> bool {
    std::env::var_os(OTEL_SDK_DISABLED).is_some_and(|val| !val.is_empty())
}

pub(crate) struct OtelExporterOtlpHeaders;

impl OtelExporterOtlpHeaders {
    /// Returns a list of key value pairs (when provided) for:
    ///  - `OTEL_EXPORTER_OTLP_HEADERS`
    ///  - `OTEL_EXPORTER_OTLP_TRACES_HEADERS`,
    ///  - `OTEL_EXPORTER_OTLP_METRICS_HEADERS`,
    ///  - `OTEL_EXPORTER_OTLP_LOGS_HEADERS`
    pub(crate) fn headers() -> HashMap<String, String> {
        let mut all_headers: HashMap<String, String> = HashMap::new();

        if let Some(otel_exporter_otlp_headers) = Self::otel_exporter_otlp_headers() {
            all_headers.extend(otel_exporter_otlp_headers);
        }

        if let Some(otel_exporter_otlp_log_headers) = Self::otel_exporter_otlp_log_headers() {
            all_headers.extend(otel_exporter_otlp_log_headers);
        }

        if let Some(otel_exporter_otlp_metrics_headers) = Self::otel_exporter_otlp_metrics_headers()
        {
            all_headers.extend(otel_exporter_otlp_metrics_headers);
        }

        if let Some(otel_exporter_otlp_traces_headers) = Self::otel_exporter_otlp_traces_headers() {
            all_headers.extend(otel_exporter_otlp_traces_headers);
        }

        all_headers
    }

    /// Returns a boolean indicating if the OTEL headers layer should be enabled.
    ///
    /// It is considered enabled if `OTEL_EXPORTER_OTLP_HEADERS` is set and not empty.
    ///
    /// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
    pub(crate) fn otel_exporter_otlp_headers() -> Option<HashMap<String, String>> {
        Self::var_get(OTEL_EXPORTER_OTLP_HEADERS)
    }

    /// Returns a boolean indicating if the OTEL log headers layer should be enabled.
    ///
    /// It is considered enabled if `OTEL_EXPORTER_OTLP_LOGS_HEADERS` is set and not empty.
    ///
    /// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
    /// Also Note this is only supported for HTTP.
    pub(crate) fn otel_exporter_otlp_log_headers() -> Option<HashMap<String, String>> {
        Self::var_get(OTEL_EXPORTER_OTLP_LOGS_HEADERS)
    }

    /// Returns a boolean indicating if the OTEL metrics headers layer should be enabled.
    ///
    /// It is considered enabled if `OTEL_EXPORTER_OTLP_METRICS_HEADERS` is set and not empty.
    ///
    /// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
    /// Also Note this is only supported for HTTP.
    pub(crate) fn otel_exporter_otlp_metrics_headers() -> Option<HashMap<String, String>> {
        Self::var_get(OTEL_EXPORTER_OTLP_METRICS_HEADERS)
    }

    /// Returns a boolean indicating if the OTEL traces headers layer should be enabled.
    ///
    /// It is considered enabled if `OTEL_EXPORTER_OTLP_TRACES_HEADERS` is set and not empty.
    ///
    /// Note that this is overridden if OTEL_SDK_DISABLED is set and not empty.
    /// Also Note this is only supported for HTTP.
    pub(crate) fn otel_exporter_otlp_traces_headers() -> Option<HashMap<String, String>> {
        Self::var_get(OTEL_EXPORTER_OTLP_TRACES_HEADERS)
    }

    fn var_get(var_name: &str) -> Option<HashMap<String, String>> {
        match std::env::var(var_name) {
            Ok(var_value) if !var_value.trim().is_empty() && !otel_sdk_disabled() => {
                let mut key_pair_header = HashMap::new();
                for pair in var_value.split(',') {
                    if let Some((k, v)) = pair.split_once('=') {
                        key_pair_header.insert(k.trim().to_owned(), v.trim().to_owned());
                    }
                }
                Some(key_pair_header)
            }
            _ => None,
        }
    }
}

/// The protocol to use for OTLP exporter.
#[derive(Debug)]
pub(crate) enum OtlpProtocol {
    Grpc,
    HttpProtobuf,
    HttpJson,
}

impl OtlpProtocol {
    /// Returns the protocol to be used for exporting traces as defined by the environment.
    pub(crate) fn traces_protocol_from_env() -> Self {
        Self::protocol_from_env(
            std::env::var(OTEL_EXPORTER_OTLP_TRACES_PROTOCOL),
            std::env::var(OTEL_EXPORTER_OTLP_PROTOCOL),
        )
    }

    /// Returns the protocol to be used for exporting metrics as defined by the environment.
    pub(crate) fn metrics_protocol_from_env() -> Self {
        Self::protocol_from_env(
            std::env::var(OTEL_EXPORTER_OTLP_METRICS_PROTOCOL),
            std::env::var(OTEL_EXPORTER_OTLP_PROTOCOL),
        )
    }

    /// Returns the protocol to be used for exporting logs as defined by the environment.
    pub(crate) fn logs_protocol_from_env() -> Self {
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
