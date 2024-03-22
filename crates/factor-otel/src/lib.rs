mod host;

use anyhow::bail;
use indexmap::IndexMap;
use opentelemetry::{
    trace::{SpanContext, SpanId, TraceContextExt},
    Context,
};
use opentelemetry_otlp::MetricExporter;
use opentelemetry_sdk::{
    logs::{log_processor_with_async_runtime::BatchLogProcessor, LogProcessor},
    resource::{EnvResourceDetector, ResourceDetector, TelemetryResourceDetector},
    runtime::Tokio,
    trace::{span_processor_with_async_runtime::BatchSpanProcessor, SpanProcessor},
    Resource,
};
use spin_factors::{Factor, FactorData, PrepareContext, RuntimeFactors, SelfInstanceBuilder};
use spin_telemetry::{
    detector::SpinResourceDetector,
    env::{otel_logs_enabled, otel_metrics_enabled, otel_tracing_enabled, OtlpProtocol},
};
use std::sync::{Arc, RwLock};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub struct OtelFactor {
    span_processor: Option<Arc<BatchSpanProcessor<Tokio>>>,
    metric_exporter: Option<Arc<MetricExporter>>,
    log_processor: Option<Arc<BatchLogProcessor<Tokio>>>,
    enable_interface: bool,
}

impl Factor for OtelFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceState;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        // Only link the WASI OTel bindings if experimental support is enabled. This means that if
        // the user tries to run a guest component that consumes the WASI OTel WIT without enabling
        // experimental support they'll see an error like "component imports instance
        // `wasi:otel/tracing@0.2.0-draft`"
        if self.enable_interface {
            ctx.link_bindings(
                spin_world::wasi::otel::tracing::add_to_linker::<_, FactorData<Self>>,
            )?;
            ctx.link_bindings(
                spin_world::wasi::otel::metrics::add_to_linker::<_, FactorData<Self>>,
            )?;
            ctx.link_bindings(spin_world::wasi::otel::logs::add_to_linker::<_, FactorData<Self>>)?;
        }
        Ok(())
    }

    fn configure_app<T: spin_factors::RuntimeFactors>(
        &self,
        _ctx: spin_factors::ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        Ok(())
    }

    fn prepare<T: spin_factors::RuntimeFactors>(
        &self,
        _: spin_factors::PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        if !self.enable_interface {
            return Ok(InstanceState::default());
        }

        // Warn the user if they enabled experimental support but didn't supply any environment variables
        if self.span_processor.is_none()
            && self.metric_exporter.is_none()
            && self.log_processor.is_none()
        {
            tracing::warn!("WASI OTel experimental support is enabled but no OTEL_EXPORTER_* environment variables were found. No telemetry will be exported.");
        }

        Ok(InstanceState {
            tracing_state: self.span_processor.as_ref().map(|span_processor| {
                Arc::new(RwLock::new(TracingState {
                    guest_span_contexts: Default::default(),
                    original_host_span_id: None,
                    span_processor: span_processor.clone(),
                }))
            }),
            metric_exporter: self.metric_exporter.clone(),
            log_processor: self.log_processor.clone(),
        })
    }
}

impl OtelFactor {
    pub fn new(spin_version: &str, enable_interface: bool) -> anyhow::Result<Self> {
        if !enable_interface {
            return Ok(Self {
                span_processor: None,
                metric_exporter: None,
                log_processor: None,
                enable_interface,
            });
        }

        let resource = Resource::builder()
            .with_detectors(&[
                // Set service.name from env OTEL_SERVICE_NAME > env OTEL_RESOURCE_ATTRIBUTES > spin
                // Set service.version from Spin metadata
                Box::new(SpinResourceDetector::new(spin_version.to_string()))
                    as Box<dyn ResourceDetector>,
                // Sets fields from env OTEL_RESOURCE_ATTRIBUTES
                Box::new(EnvResourceDetector::new()),
                // Sets telemetry.sdk{name, language, version}
                Box::new(TelemetryResourceDetector),
            ])
            .build();

        let span_processor = if otel_tracing_enabled() {
            // This will configure the exporter based on the OTEL_EXPORTER_* environment variables.
            let span_exporter = match OtlpProtocol::traces_protocol_from_env() {
                OtlpProtocol::Grpc => opentelemetry_otlp::SpanExporter::builder()
                    .with_tonic()
                    .build()?,
                OtlpProtocol::HttpProtobuf => opentelemetry_otlp::SpanExporter::builder()
                    .with_http()
                    .build()?,
                OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
            };

            let mut span_processor = BatchSpanProcessor::builder(span_exporter, Tokio).build();
            span_processor.set_resource(&resource);
            Some(Arc::new(span_processor))
        } else {
            None
        };

        let metric_exporter = if otel_metrics_enabled() {
            let metric_exporter = match OtlpProtocol::metrics_protocol_from_env() {
                OtlpProtocol::Grpc => opentelemetry_otlp::MetricExporter::builder()
                    .with_tonic()
                    .build()?,
                OtlpProtocol::HttpProtobuf => opentelemetry_otlp::MetricExporter::builder()
                    .with_http()
                    .build()?,
                OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
            };
            Some(Arc::new(metric_exporter))
        } else {
            None
        };

        let log_processor = if otel_logs_enabled() {
            let log_exporter = match OtlpProtocol::logs_protocol_from_env() {
                OtlpProtocol::Grpc => opentelemetry_otlp::LogExporter::builder()
                    .with_tonic()
                    .build()?,
                OtlpProtocol::HttpProtobuf => opentelemetry_otlp::LogExporter::builder()
                    .with_http()
                    .build()?,
                OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
            };

            let log_processor = BatchLogProcessor::builder(log_exporter, Tokio).build();
            log_processor.set_resource(&resource);
            Some(Arc::new(log_processor))
        } else {
            None
        };

        Ok(Self {
            span_processor,
            metric_exporter,
            log_processor,
            enable_interface,
        })
    }
}

#[derive(Default)]
pub struct InstanceState {
    tracing_state: Option<Arc<RwLock<TracingState>>>,
    metric_exporter: Option<Arc<MetricExporter>>,
    log_processor: Option<Arc<BatchLogProcessor<Tokio>>>,
}

impl SelfInstanceBuilder for InstanceState {}

/// Internal tracing state of the OtelFactor InstanceState.
///
/// This data lives here rather than directly on InstanceState so that we can have multiple things
/// take Arc references to it and so that if tracing is disabled we don't keep doing needless
/// bookkeeping of host spans.
pub(crate) struct TracingState {
    /// An order-preserved mapping between immutable [SpanId]s of guest created spans and their
    /// corresponding [SpanContext].
    ///
    /// The topmost [SpanId] is the last active span. When a span is ended it is removed from this
    /// map (regardless of whether it is the active span) and all other spans are shifted back to
    /// retain relative order.
    pub(crate) guest_span_contexts: IndexMap<SpanId, SpanContext>,

    /// Id of the last span emitted from within the host before entering the guest.
    ///
    /// We use this to avoid accidentally reparenting the original host span as a child of a guest
    /// span.
    pub(crate) original_host_span_id: Option<SpanId>,

    /// The span processor used to export spans.
    span_processor: Arc<BatchSpanProcessor<Tokio>>,
}

/// Manages access to the OtelFactor tracing state for the purpose of maintaining proper span
/// parent/child relationships when WASI Otel spans are being created.
#[derive(Default)]
pub struct OtelFactorState {
    pub(crate) tracing_state: Option<Arc<RwLock<TracingState>>>,
}

impl OtelFactorState {
    /// Creates an [`OtelFactorState`] from a [`PrepareContext`].
    ///
    /// If [`RuntimeFactors`] does not contain an [`OtelFactor`], then calling
    /// [`OtelFactorState::reparent_tracing_span`] will be a no-op.
    pub fn from_prepare_context<T: RuntimeFactors, F: Factor>(
        prepare_context: &mut PrepareContext<T, F>,
    ) -> anyhow::Result<Self> {
        let tracing_state = match prepare_context.instance_builder::<OtelFactor>() {
            Ok(instance_state) => instance_state.tracing_state.clone(),
            Err(spin_factors::Error::NoSuchFactor(_)) => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self { tracing_state })
    }

    /// Reparents the current [tracing] span to be a child of the last active guest span.
    ///
    /// The otel factor enables guests to emit spans that should be part of the same trace as the
    /// host is producing for a request. Below is an example trace. A request is made to an app, a
    /// guest span is created and then the host is re-entered to fetch a key value.
    ///
    /// ```text
    /// | GET /... _________________________________|
    ///    | execute_wasm_component foo ___________|
    ///       | my_guest_span ___________________|
    ///          | spin_key_value.get |
    /// ```
    ///
    /// Setting the guest spans parent as the host is enabled through current_span_context.
    /// However, the more difficult task is having the host factor spans be children of the guest
    /// span. [`OtelFactorState::reparent_tracing_span`] handles this by reparenting the current span to
    /// be a child of the last active guest span (which is tracked internally in the otel factor).
    ///
    /// Note that if the otel factor is not in your [`RuntimeFactors`] than this is effectively a
    /// no-op.
    ///
    /// This MUST only be called from a factor host implementation function that is instrumented.
    ///
    /// This MUST be called at the very start of the function before any awaits.
    pub fn reparent_tracing_span(&self) {
        // If tracing_state is None then tracing is not enabled so we should return early
        let tracing_state = if let Some(state) = self.tracing_state.as_ref() {
            state.read().unwrap()
        } else {
            return;
        };

        // If there are no active guest spans then there is nothing to do
        let Some((_, active_span_context)) = tracing_state.guest_span_contexts.last() else {
            return;
        };

        // Ensure that we are not reparenting the original host span
        if let Some(original_host_span_id) = tracing_state.original_host_span_id {
            debug_assert_ne!(
                &original_host_span_id,
                &tracing::Span::current()
                    .context()
                    .span()
                    .span_context()
                    .span_id(),
                    "Incorrectly attempting to reparent the original host span. Likely `reparent_tracing_span` was called in an incorrect location."
            );
        }

        // Now reparent the current span to the last active guest span
        let parent_context = Context::new().with_remote_span_context(active_span_context.clone());
        tracing::Span::current().set_parent(parent_context);
    }
}
