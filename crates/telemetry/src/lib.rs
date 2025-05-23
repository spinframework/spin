use std::io::IsTerminal;

use anyhow::Context;
use env::otel_logs_enabled;
use env::otel_metrics_enabled;
use env::otel_tracing_enabled;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing_subscriber::{fmt, prelude::*, registry, EnvFilter, Layer};

mod alert_in_dev;
pub mod detector;
mod env;
pub mod logs;
pub mod metrics;
mod propagation;
mod traces;

pub use propagation::extract_trace_context;
pub use propagation::inject_trace_context;

/// Initializes telemetry for Spin using the [tracing] library.
///
/// Under the hood this involves initializing a [tracing::Subscriber] with multiple [Layer]s. One
/// [Layer] emits [tracing] events to stderr, another sends spans to an OTel collector, and another
/// sends metrics to an OTel collector.
///
/// Configuration for the OTel layers is pulled from the environment.
///
/// Examples of emitting traces from Spin:
///
/// ```no_run
/// # use tracing::instrument;
/// # use tracing::Level;
/// #[instrument(name = "span_name", err(level = Level::INFO), fields(otel.name = "dynamically set name"))]
/// fn func_you_want_to_trace() -> anyhow::Result<String> {
///     Ok("Hello, world!".to_string())
/// }
/// ```
///
/// Some notes on tracing:
///
/// - If you don't want the span to be collected by default emit it at a trace or debug level.
/// - Make sure you `.in_current_span()` any spawned tasks so the span context is propagated.
/// - Use the otel.name attribute to dynamically set the span name.
/// - Use the err argument to have instrument automatically handle errors.
///
/// Examples of emitting metrics from Spin:
///
/// ```no_run
/// spin_telemetry::metrics::monotonic_counter!(spin.metric_name = 1, metric_attribute = "value");
/// ```
pub fn init(spin_version: String) -> anyhow::Result<()> {
    // This layer will print all tracing library log messages to stderr.
    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_filter(
            // Filter directives explained here https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives
            EnvFilter::from_default_env()
                // Wasmtime is too noisy
                .add_directive("wasmtime_wasi_http=warn".parse()?)
                // Watchexec is too noisy
                .add_directive("watchexec=off".parse()?)
                // We don't want to duplicate application logs
                .add_directive("[{app_log}]=off".parse()?)
                .add_directive("[{app_log_non_utf8}]=off".parse()?),
        );

    let otel_tracing_layer = if otel_tracing_enabled() {
        Some(
            traces::otel_tracing_layer(spin_version.clone())
                .context("failed to initialize otel tracing")?,
        )
    } else {
        None
    };

    let otel_metrics_layer = if otel_metrics_enabled() {
        Some(
            metrics::otel_metrics_layer(spin_version.clone())
                .context("failed to initialize otel metrics")?,
        )
    } else {
        None
    };

    let alert_in_dev_layer = alert_in_dev::alert_in_dev_layer();

    // Build a registry subscriber with the layers we want to use.
    registry()
        .with(otel_tracing_layer)
        .with(otel_metrics_layer)
        .with(fmt_layer)
        .with(alert_in_dev_layer)
        .init();

    // Used to propagate trace information in the standard W3C TraceContext format. Even if the otel
    // layer is disabled we still want to propagate trace context.
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    if otel_logs_enabled() {
        logs::init_otel_logging_backend(spin_version)
            .context("failed to initialize otel logging")?;
    }

    Ok(())
}
