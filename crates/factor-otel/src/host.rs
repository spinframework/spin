use crate::InstanceState;
use anyhow::anyhow;
use anyhow::Result;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::logs::LogProcessor;
use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
use opentelemetry_sdk::trace::SpanProcessor;
use spin_world::wasi;
use tracing_opentelemetry::OpenTelemetrySpanExt;

impl wasi::otel::tracing::Host for InstanceState {
    async fn on_start(&mut self, context: wasi::otel::tracing::SpanContext) -> Result<()> {
        // If the host does not have tracing enabled we just no-op
        let Some(tracing_state) = self.tracing_state.as_ref() else {
            return Ok(());
        };
        let mut tracing_state = tracing_state.write().unwrap();

        // Before we do anything make sure we track the original host span ID for reparenting
        if tracing_state.original_host_span_id.is_none() {
            tracing_state.original_host_span_id = Some(
                tracing::Span::current()
                    .context()
                    .span()
                    .span_context()
                    .span_id(),
            );
        }

        // Track the guest spans context in our ordered map
        let span_context: opentelemetry::trace::SpanContext = context.into();
        tracing_state
            .guest_span_contexts
            .insert(span_context.span_id(), span_context);

        Ok(())
    }

    async fn on_end(&mut self, span_data: wasi::otel::tracing::SpanData) -> Result<()> {
        // If the host does not have tracing enabled we just no-op
        let Some(tracing_state) = self.tracing_state.as_ref() else {
            return Ok(());
        };
        let mut tracing_state = tracing_state.write().unwrap();

        let span_context: opentelemetry::trace::SpanContext = span_data.span_context.clone().into();
        let span_id: opentelemetry::trace::SpanId = span_context.span_id();

        if tracing_state
            .guest_span_contexts
            .shift_remove(&span_id)
            .is_none()
        {
            Err(anyhow!("Trying to end a span that was not started"))?;
        }

        tracing_state.span_processor.on_end(span_data.into());

        Ok(())
    }

    async fn outer_span_context(&mut self) -> Result<wasi::otel::tracing::SpanContext> {
        Ok(tracing::Span::current()
            .context()
            .span()
            .span_context()
            .clone()
            .into())
    }
}

impl wasi::otel::metrics::Host for InstanceState {
    async fn export(
        &mut self,
        metrics: wasi::otel::metrics::ResourceMetrics,
    ) -> spin_core::wasmtime::Result<std::result::Result<(), wasi::otel::metrics::Error>> {
        // If the host does not have metrics enabled we just no-op
        let Some(metric_exporter) = self.metric_exporter.as_ref() else {
            return Ok(Ok(()));
        };

        match metric_exporter.export(&mut metrics.into()).await {
            Ok(_) => Ok(Ok(())),
            Err(e) => match e {
                OTelSdkError::AlreadyShutdown => {
                    let msg = "Shutdown has already been invoked";
                    tracing::error!(msg);
                    Ok(Err(msg.to_string()))
                }
                OTelSdkError::InternalFailure(e) => {
                    let detailed_msg = format!("Internal failure: {}", e);
                    tracing::error!(detailed_msg);
                    Ok(Err("Internal failure.".to_string()))
                }
                OTelSdkError::Timeout(d) => {
                    let detailed_msg = format!("Operation timed out after {} seconds", d.as_secs());
                    tracing::error!(detailed_msg);
                    Ok(Err("Operation timed out.".to_string()))
                }
            },
        }
    }
}

impl wasi::otel::logs::Host for InstanceState {
    async fn on_emit(
        &mut self,
        data: wasi::otel::logs::LogRecord,
    ) -> spin_core::wasmtime::Result<()> {
        // If the host does not have logs enabled we just no-op
        let Some(log_processor) = self.log_processor.as_ref() else {
            return Ok(());
        };

        let (mut record, scope) = spin_world::wasi_otel::parse_wasi_log_record(data);
        log_processor.emit(&mut record, &scope);
        Ok(())
    }
}
