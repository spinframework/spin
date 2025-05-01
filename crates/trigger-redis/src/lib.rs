use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use futures::{StreamExt, TryFutureExt};
use redis::{Client, Msg};
use serde::Deserialize;
use spin_factor_variables::VariablesFactor;
use spin_factors::RuntimeFactors;
use spin_trigger::{cli::NoGlobalConfig, App, Trigger, TriggerApp};
use spin_world::exports::fermyon::spin::inbound_redis;
use tracing::{instrument, Level};

pub struct RedisTrigger;

/// Redis trigger metadata.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerMetadata {
    address: String,
}

/// Redis trigger configuration.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerConfig {
    /// Component ID to invoke
    component: String,
    /// Channel to subscribe to
    channel: String,
    /// Optionally override address for trigger
    address: Option<String>,
}

impl<F: RuntimeFactors> Trigger<F> for RedisTrigger {
    const TYPE: &'static str = "redis";

    type GlobalConfig = NoGlobalConfig;

    type InstanceState = ();

    fn new(_cfg: Self::GlobalConfig, _app: &App) -> anyhow::Result<Self> {
        Ok(Self)
    }

    async fn run(self, trigger_app: spin_trigger::TriggerApp<Self, F>) -> anyhow::Result<()> {
        let app_variables = trigger_app
            .configured_app()
            .app_state::<VariablesFactor>()
            .context("RedisTrigger depends on VariablesFactor")?;

        let app = trigger_app.app();
        let trigger_type = <Self as Trigger<F>>::TYPE;
        let metadata = app
            .get_trigger_metadata::<TriggerMetadata>(trigger_type)?
            .unwrap_or_default();
        let default_address_expr = &metadata.address;
        let default_address = app_variables
            .resolve_expression(default_address_expr.clone())
            .await
            .with_context(|| {
                format!("failed to resolve redis trigger default address {default_address_expr:?}")
            })?;

        // Maps <server address> -> <channel> -> <component IDs>
        let mut server_channel_components: HashMap<String, ChannelComponents> = HashMap::new();

        // Resolve trigger configs before starting any subscribers
        for (_, config) in app
            .trigger_configs::<TriggerConfig>(trigger_type)?
            .into_iter()
            .collect::<Vec<_>>()
        {
            let component_id = config.component;

            let address_expr = config.address.as_ref().unwrap_or(&default_address);
            let address = app_variables
                .resolve_expression(address_expr.clone())
                .await
                .with_context(|| {
                    format!(
                        "failed to resolve redis trigger address {address_expr:?} for component {component_id}"
                    )
                })?;

            let channel_expr = &config.channel;
            let channel = app_variables
                .resolve_expression(channel_expr.clone())
                .await
                .with_context(|| {
                    format!(
                        "failed to resolve redis trigger channel {channel_expr:?} for component {component_id}"
                    )
                })?;

            server_channel_components
                .entry(address)
                .or_default()
                .entry(channel)
                .or_default()
                .push(component_id);
        }

        // Start subscriber(s)
        let trigger_app = Arc::new(trigger_app);
        let mut subscriber_tasks = Vec::new();
        for (address, channel_components) in server_channel_components {
            let subscriber = Subscriber::new(address, trigger_app.clone(), channel_components)?;
            let task = tokio::spawn(subscriber.run_listener());
            subscriber_tasks.push(task);
        }

        // Wait for any task to complete
        let (res, _, _) = futures::future::select_all(subscriber_tasks).await;
        res?
    }
}

/// Maps <channel> -> <component IDs>
type ChannelComponents = HashMap<String, Vec<String>>;

/// Subscribes to channels from a single Redis server.
struct Subscriber<F: RuntimeFactors> {
    client: Client,
    trigger_app: Arc<TriggerApp<RedisTrigger, F>>,
    channel_components: ChannelComponents,
}

impl<F: RuntimeFactors> Subscriber<F> {
    fn new(
        address: String,
        trigger_app: Arc<TriggerApp<RedisTrigger, F>>,
        channel_components: ChannelComponents,
    ) -> anyhow::Result<Self> {
        let client = Client::open(address)?;
        Ok(Self {
            client,
            trigger_app,
            channel_components,
        })
    }

    async fn run_listener(self) -> anyhow::Result<()> {
        let server_addr = &self.client.get_connection_info().addr;

        tracing::info!("Connecting to Redis server at {server_addr}");
        let mut pubsub = self
            .client
            .get_async_pubsub()
            .await
            .with_context(|| format!("Redis trigger failed to connect to {server_addr}"))?;

        println!("Active Channels on {server_addr}:");

        // Subscribe to channels
        for (channel, components) in &self.channel_components {
            tracing::info!("Subscribing to {channel:?} on {server_addr}");
            pubsub.subscribe(channel).await.with_context(|| {
                format!("Redis trigger failed to subscribe to channel {channel:?} on {server_addr}")
            })?;
            println!("\t{server_addr}/{channel}: [{}]", components.join(","));
        }

        let mut message_stream = pubsub.on_message();
        while let Some(msg) = message_stream.next().await {
            if let Err(err) = self.handle_message(msg).await {
                tracing::error!("Error handling message from {server_addr}: {err}");
            }
        }
        Err(anyhow::anyhow!("disconnected from {server_addr}"))
    }

    #[instrument(name = "spin_trigger_redis.handle_message", skip_all, err(level = Level::INFO), fields(
        otel.name = format!("{} receive", msg.get_channel_name()),
        otel.kind = "consumer",
        messaging.operation = "receive",
        messaging.system = "redis"
    ))]
    async fn handle_message(&self, msg: Msg) -> anyhow::Result<()> {
        let server_addr = &self.client.get_connection_info().addr;
        let channel = msg.get_channel_name();
        tracing::trace!(%server_addr, %channel, "Received message");

        let Some(component_ids) = self.channel_components.get(channel) else {
            anyhow::bail!("message from unexpected channel {channel:?}");
        };

        let dispatch_futures = component_ids.iter().map(|component_id| {
            tracing::trace!("Executing Redis component {component_id}");
            self.dispatch_handler(&msg, component_id)
                .inspect_err(move |err| {
                    tracing::info!("Component {component_id} handler failed: {err}");
                })
        });
        futures::future::join_all(dispatch_futures).await;

        Ok(())
    }

    async fn dispatch_handler(&self, msg: &Msg, component_id: &str) -> anyhow::Result<()> {
        spin_telemetry::metrics::monotonic_counter!(
            spin.request_count = 1,
            trigger_type = "redis",
            app_id = self.trigger_app.app().id(),
            component_id = component_id
        );

        let (instance, mut store) = self
            .trigger_app
            .prepare(component_id)?
            .instantiate(())
            .await?;

        let guest_indices = inbound_redis::GuestIndices::new_instance(&mut store, &instance)?;
        let guest = guest_indices.load(&mut store, &instance)?;

        let payload = msg.get_payload_bytes().to_vec();

        guest
            .call_handle_message(&mut store, &payload)
            .await?
            .context("Redis handler returned an error")
    }
}
