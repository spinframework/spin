use spin_core::async_trait;
use spin_factors::RuntimeFactors;
use spin_factors_executor::{ExecutorHooks, FactorsInstanceBuilder};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

static GLOBAL_METRICS_TRACKER: OnceLock<MetricsTracker> = OnceLock::new();

#[derive(Clone)]
pub struct ComponentMetrics {
    pub memory_usage_init: Vec<u64>,
    pub memory_usage_exec: Vec<u64>,
    pub cpu_time_elapsed: Vec<Duration>,
}

#[derive(Clone)]
pub struct MetricsTracker {
    data: Arc<Mutex<HashMap<String, ComponentMetrics>>>,
}

impl MetricsTracker {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn global() -> &'static MetricsTracker {
        GLOBAL_METRICS_TRACKER.get_or_init(|| MetricsTracker::new())
    }

    pub fn get_data(&self) -> HashMap<String, ComponentMetrics> {
        self.data.lock().unwrap().clone()
    }

    pub fn set_memory_usage_init(&self, component_id: String, usage: u64) {
        let mut map = self.data.lock().unwrap();
        map.entry(component_id)
            .or_insert_with(|| ComponentMetrics {
                memory_usage_init: vec![],
                memory_usage_exec: vec![],
                cpu_time_elapsed: vec![],
            })
            .memory_usage_init
            .push(usage);
    }

    pub fn set_memory_usage_exec(&self, component_id: String, usage: u64) {
        let mut map = self.data.lock().unwrap();
        map.entry(component_id)
            .or_insert_with(|| ComponentMetrics {
                memory_usage_init: vec![],
                memory_usage_exec: vec![],
                cpu_time_elapsed: vec![],
            })
            .memory_usage_exec
            .push(usage);
    }

    pub fn set_cpu_time_elapsed(&self, component_id: String, elapsed_time: Duration) {
        let mut map = self.data.lock().unwrap();
        map.entry(component_id)
            .or_insert_with(|| ComponentMetrics {
                memory_usage_init: vec![],
                memory_usage_exec: vec![],
                cpu_time_elapsed: vec![],
            })
            .cpu_time_elapsed
            .push(elapsed_time);
    }
}

impl Default for MetricsTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MetricsTrackerHook;
impl MetricsTrackerHook {
    pub fn new() -> Self {
        Self
    }

    pub fn tracker() -> &'static MetricsTracker {
        MetricsTracker::global()
    }
}

#[async_trait]
impl<F: RuntimeFactors, U> ExecutorHooks<F, U> for MetricsTrackerHook {
    fn prepare_instance(&self, _builder: &mut FactorsInstanceBuilder<F, U>) -> anyhow::Result<()> {
        Ok(())
    }
}
