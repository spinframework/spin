mod host;

use anyhow::bail;
use indexmap::IndexMap;
use opentelemetry::{
    trace::{SpanContext, SpanId, TraceContextExt},
    Context,
};
use opentelemetry_otlp::MetricExporter;
use opentelemetry_sdk::{
    resource::{EnvResourceDetector, ResourceDetector, TelemetryResourceDetector},
    runtime::Tokio,
    trace::{span_processor_with_async_runtime::BatchSpanProcessor, SpanProcessor},
    Resource,
};
use spin_factors::{Factor, FactorData, PrepareContext, RuntimeFactors, SelfInstanceBuilder};
use spin_telemetry::{detector::SpinResourceDetector, env::OtlpProtocol};
use std::sync::{Arc, RwLock};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub struct OtelFactor {
    span_processor: Arc<BatchSpanProcessor<Tokio>>,
    metric_exporter: Arc<MetricExporter>,
}

impl Factor for OtelFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceState;

    fn init(&mut self, ctx: &mut impl spin_factors::InitContext<Self>) -> anyhow::Result<()> {
        ctx.link_bindings(spin_world::wasi::otel::tracing::add_to_linker::<_, FactorData<Self>>)?;
        ctx.link_bindings(spin_world::wasi::otel::metrics::add_to_linker::<_, FactorData<Self>>)?;
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
        Ok(InstanceState {
            state: Arc::new(RwLock::new(State {
                guest_span_contexts: Default::default(),
                original_host_span_id: None,
            })),
            span_processor: self.span_processor.clone(),
            metric_exporter: self.metric_exporter.clone(),
        })
    }
}

impl OtelFactor {
    pub fn new() -> anyhow::Result<Self> {
        // This is a hack b/c we know the version of this crate will be the same as the version of Spin
        let spin_version = env!("CARGO_PKG_VERSION").to_string();

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

        let metric_exporter = match OtlpProtocol::metrics_protocol_from_env() {
            OtlpProtocol::Grpc => opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .build()?,
            OtlpProtocol::HttpProtobuf => opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .build()?,
            OtlpProtocol::HttpJson => bail!("http/json OTLP protocol is not supported"),
        };

        Ok(Self {
            span_processor: Arc::new(span_processor),
            metric_exporter: Arc::new(metric_exporter),
        })
    }
}

pub struct InstanceState {
    pub(crate) state: Arc<RwLock<State>>,
    pub(crate) span_processor: Arc<BatchSpanProcessor<Tokio>>,
    pub(crate) metric_exporter: Arc<MetricExporter>,
}

impl SelfInstanceBuilder for InstanceState {}

/// Internal state of the OtelFactor instance state.
///
/// This data lives here rather than directly on InstanceState so that we can have multiple things
/// take Arc references to it.
pub(crate) struct State {
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
}

/// Manages access to the OtelFactor state for the purpose of maintaining proper span
/// parent/child relationships when WASI Otel spans are being created.
pub struct OtelContext {
    pub(crate) state: Option<Arc<RwLock<State>>>,
}

impl OtelContext {
    /// Creates an [`OtelContext`] from a [`PrepareContext`].
    ///
    /// If [`RuntimeFactors`] does not contain an [`OtelFactor`], then calling
    /// [`OtelContext::reparent_tracing_span`] will be a no-op.
    pub fn from_prepare_context<T: RuntimeFactors, F: Factor>(
        prepare_context: &mut PrepareContext<T, F>,
    ) -> anyhow::Result<Self> {
        let state = match prepare_context.instance_builder::<OtelFactor>() {
            Ok(instance_state) => Some(instance_state.state.clone()),
            Err(spin_factors::Error::NoSuchFactor(_)) => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self { state })
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
    ///  Setting the guest spans parent as the host is enabled through current_span_context.
    /// However, the more difficult task is having the host factor spans be children of the guest
    /// span. [`OtelContext::reparent_tracing_span`] handles this by reparenting the current span to
    /// be a child of the last active guest span (which is tracked internally in the otel factor).
    ///
    /// Note that if the otel factor is not in your [`RuntimeFactors`] than this is effectively a
    /// no-op.
    ///
    /// This MUST only be called from a factor host implementation function that is instrumented.
    ///
    /// This MUST be called at the very start of the function before any awaits.
    pub fn reparent_tracing_span(&self) {
        // If state is None then we want to return early b/c the factor doesn't depend on the
        // Otel factor and therefore there is nothing to do
        let state = if let Some(state) = self.state.as_ref() {
            state.read().unwrap()
        } else {
            return;
        };

        // If there are no active guest spans then there is nothing to do
        let Some((_, active_span_context)) = state.guest_span_contexts.last() else {
            return;
        };

        // Ensure that we are not reparenting the original host span
        if let Some(original_host_span_id) = state.original_host_span_id {
            if tracing::Span::current()
                .context()
                .span()
                .span_context()
                .span_id()
                .eq(&original_host_span_id)
            {
                panic!("Incorrectly attempting to reparent the original host span. Likely `reparent_tracing_span` was called in an incorrect location.")
            }
        }

        // Now reparent the current span to the last active guest span
        let parent_context = Context::new().with_remote_span_context(active_span_context.clone());
        tracing::Span::current().set_parent(parent_context);
    }
}
