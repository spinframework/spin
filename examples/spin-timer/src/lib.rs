use std::collections::HashMap;

use clap::Args;
use serde::{Deserialize, Serialize};
use spin_factors::RuntimeFactors;
use spin_trigger::{App, Trigger, TriggerApp};

wasmtime::component::bindgen!({
    path: ".",
    world: "spin-timer",
    imports: { default: async },
    exports: { default: async },
});

#[derive(Args)]
pub struct CliArgs {
    /// If true, run each component once and exit
    #[clap(long)]
    pub test: bool,
}

// The trigger structure with all values processed and ready
pub struct TimerTrigger {
    test: bool,
    speedup: u64,
    component_timings: HashMap<String, u64>,
}

// Picks out the timer entry from the application-level trigger settings
#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct TriggerMetadataParent {
    timer: Option<TriggerMetadata>,
}

// Application-level settings (raw serialization format)
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TriggerMetadata {
    speedup: Option<u64>,
}

// Per-component settings (raw serialization format)
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimerTriggerConfig {
    component: String,
    interval_secs: u64,
}

impl<F: RuntimeFactors> Trigger<F> for TimerTrigger {
    const TYPE: &'static str = "timer";

    type CliArgs = CliArgs;

    type InstanceState = ();

    fn new(cli_args: Self::CliArgs, app: &App) -> anyhow::Result<Self> {
        let trigger_type = <Self as Trigger<F>>::TYPE;
        let metadata = app
            .get_trigger_metadata::<TriggerMetadata>(trigger_type)?
            .unwrap_or_default();
        let speedup = metadata.speedup.unwrap_or(1);

        let component_timings = app
            .trigger_configs::<TimerTriggerConfig>(trigger_type)?
            .into_iter()
            .map(|(_, config)| (config.component.clone(), config.interval_secs))
            .collect();

        Ok(Self {
            test: cli_args.test,
            speedup,
            component_timings,
        })
    }

    async fn run(self, trigger_app: TriggerApp<Self, F>) -> anyhow::Result<()> {
        if self.test {
            for component in self.component_timings.keys() {
                self.handle_timer_event(&trigger_app, component).await?;
            }
        } else {
            // This trigger spawns threads, which Ctrl+C does not kill.  So
            // for this case we need to detect Ctrl+C and shut those threads
            // down.  For simplicity, we do this by terminating the process.
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.unwrap();
                std::process::exit(0);
            });

            let speedup = self.speedup;
            tokio_scoped::scope(|scope| {
                // For each component, run its own timer loop
                for (component_id, interval_secs) in &self.component_timings {
                    scope.spawn(async {
                        let duration =
                            tokio::time::Duration::from_millis(*interval_secs * 1000 / speedup);
                        loop {
                            tokio::time::sleep(duration).await;

                            self.handle_timer_event(&trigger_app, component_id)
                                .await
                                .unwrap();
                        }
                    });
                }
            });
        }
        Ok(())
    }
}

impl TimerTrigger {
    async fn handle_timer_event<F: RuntimeFactors>(
        &self,
        trigger_app: &TriggerApp<Self, F>,
        component_id: &str,
    ) -> anyhow::Result<()> {
        let instance_builder = trigger_app.prepare(component_id)?;
        let (instance, mut store) = instance_builder.instantiate(()).await?;
        let timer = SpinTimer::new(&mut store, &instance)?;
        timer.call_handle_timer_request(&mut store).await
    }
}
