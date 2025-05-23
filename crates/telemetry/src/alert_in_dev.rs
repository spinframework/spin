//! Provides a way for warnings to be force-raised in the
//! `spin up` environment even if RUST_LOG is not set to warn.
//! This is useful for things that are not errors but where we
//! want application developers to know they have a problem.

use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;

const ALERT_IN_DEV_TAG: &str = "alert_in_dev";

/// A layer which prints a terminal warning (using [terminal::warn!]) if
/// a trace event contains the tag "alert_in_dev" (with any value).
pub(crate) fn alert_in_dev_layer<S: Subscriber>() -> impl Layer<S> {
    CommandLineAlertingLayer
}

pub struct CommandLineAlertingLayer;

impl<S: Subscriber> Layer<S> for CommandLineAlertingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata().fields();
        if meta.field(ALERT_IN_DEV_TAG).is_some() {
            warn(event);
        }
    }
}

fn warn(event: &Event<'_>) {
    let mut visitor = PrintMessageAsWarning;
    event.record(&mut visitor);
}

struct PrintMessageAsWarning;

impl tracing::field::Visit for PrintMessageAsWarning {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            terminal::warn!("{value:?}");
        }
    }
}
