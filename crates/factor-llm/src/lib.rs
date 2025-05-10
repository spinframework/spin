mod host;
pub mod spin;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use spin_factors::{
    ConfigureAppContext, Factor, PrepareContext, RuntimeFactors, SelfInstanceBuilder,
};
use spin_locked_app::MetadataKey;
use spin_world::v1::llm::{self as v1};
use spin_world::v2::llm::{self as v2};
use tokio::sync::Mutex;

pub const ALLOWED_MODELS_KEY: MetadataKey<Vec<String>> = MetadataKey::new("ai_models");

/// The factor for LLMs.
pub struct LlmFactor {
    default_engine_creator: Box<dyn LlmEngineCreator>,
}

impl LlmFactor {
    /// Creates a new LLM factor with the given default engine creator.
    ///
    /// The default engine creator is used to create the engine if no runtime configuration is provided.
    pub fn new<F: LlmEngineCreator + 'static>(default_engine_creator: F) -> Self {
        Self {
            default_engine_creator: Box::new(default_engine_creator),
        }
    }
}

impl Factor for LlmFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceState;

    fn init<C>(&mut self, ctx: &mut C) -> anyhow::Result<()>
    where
        C: spin_factors::InitContext<Self>,
    {
        ctx.link_bindings(spin_world::v1::llm::add_to_linker)?;
        ctx.link_bindings(spin_world::v2::llm::add_to_linker)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let component_allowed_models = ctx
            .app()
            .components()
            .map(|component| {
                Ok((
                    component.id().to_string(),
                    component
                        .get_metadata(ALLOWED_MODELS_KEY)?
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<HashSet<_>>()
                        .into(),
                ))
            })
            .collect::<anyhow::Result<_>>()?;
        let engine = ctx
            .take_runtime_config()
            .map(|c| c.engine)
            .unwrap_or_else(|| self.default_engine_creator.create());
        Ok(AppState {
            engine,
            component_allowed_models,
        })
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<Self::InstanceBuilder> {
        let allowed_models = ctx
            .app_state()
            .component_allowed_models
            .get(ctx.app_component().id())
            .cloned()
            .unwrap_or_default();
        let engine = ctx.app_state().engine.clone();

        Ok(InstanceState {
            engine,
            allowed_models,
        })
    }
}

/// The application state for the LLM factor.
pub struct AppState {
    engine: Arc<Mutex<dyn LlmEngine>>,
    component_allowed_models: HashMap<String, Arc<HashSet<String>>>,
}

/// The instance state for the LLM factor.
pub struct InstanceState {
    engine: Arc<Mutex<dyn LlmEngine>>,
    pub allowed_models: Arc<HashSet<String>>,
}

/// The runtime configuration for the LLM factor.
pub struct RuntimeConfig {
    engine: Arc<Mutex<dyn LlmEngine>>,
}

impl SelfInstanceBuilder for InstanceState {}

/// The interface for a language model engine.
#[async_trait]
pub trait LlmEngine: Send + Sync {
    async fn infer(
        &mut self,
        model: v1::InferencingModel,
        prompt: String,
        params: v2::InferencingParams,
    ) -> Result<v2::InferencingResult, v2::Error>;

    async fn generate_embeddings(
        &mut self,
        model: v2::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<v2::EmbeddingsResult, v2::Error>;

    /// A human-readable summary of the given engine's configuration
    ///
    /// Example: "local model"
    fn summary(&self) -> Option<String> {
        None
    }
}

/// A creator for an LLM engine.
pub trait LlmEngineCreator: Send + Sync {
    fn create(&self) -> Arc<Mutex<dyn LlmEngine>>;
}

impl<F> LlmEngineCreator for F
where
    F: Fn() -> Arc<Mutex<dyn LlmEngine>> + Send + Sync,
{
    fn create(&self) -> Arc<Mutex<dyn LlmEngine>> {
        self()
    }
}
