use std::env;

use opentelemetry::{Key, KeyValue, Value};
use opentelemetry_sdk::{
    Resource,
    resource::{EnvResourceDetector, ResourceDetector},
};

const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// Custom resource detector for Spin relevant attributes service.name and service.version.
///
/// To set service.name this detector will first try `OTEL_SERVICE_NAME` env. If it's not available,
/// then it will check the `OTEL_RESOURCE_ATTRIBUTES` env and see if it contains `service.name`
/// resource. If it's also not available, it will use `spin`.
///
/// To set service.version, it will use the spin_version passed in new.
#[derive(Debug)]
pub struct SpinResourceDetector {
    spin_version: String,
}

impl SpinResourceDetector {
    /// Create a new instance of SpinResourceDetector.
    pub fn new(spin_version: String) -> Self {
        SpinResourceDetector { spin_version }
    }
}

impl ResourceDetector for SpinResourceDetector {
    fn detect(&self) -> Resource {
        let service_name = env::var(OTEL_SERVICE_NAME)
            .ok()
            .filter(|s| !s.is_empty())
            .map(Value::from)
            .or_else(|| {
                EnvResourceDetector::new()
                    .detect()
                    .get(&Key::new("service.name"))
            })
            .unwrap_or_else(|| "spin".into());
        Resource::builder()
            .with_attributes(vec![
                KeyValue::new("service.name", service_name),
                KeyValue::new("service.version", self.spin_version.clone()),
            ])
            .build()
    }
}
