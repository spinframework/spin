use std::path::PathBuf;
use std::sync::Arc;

use spin_factors::runtime_config::toml::GetTomlValue;
use spin_llm_remote_http::RemoteHttpLlmEngine;
use spin_world::async_trait;
use spin_world::v1::llm::{self as v1};
use spin_world::v2::llm::{self as v2};
use tokio::sync::Mutex;
use url::Url;

use crate::{LlmEngine, LlmEngineCreator, RuntimeConfig};

#[cfg(feature = "llm")]
mod local {
    use super::*;
    pub use spin_llm_local::LocalLlmEngine;

    #[async_trait]
    impl LlmEngine for LocalLlmEngine {
        async fn infer(
            &mut self,
            model: v2::InferencingModel,
            prompt: String,
            params: v2::InferencingParams,
        ) -> Result<v2::InferencingResult, v2::Error> {
            self.infer(model, prompt, params).await
        }

        async fn generate_embeddings(
            &mut self,
            model: v2::EmbeddingModel,
            data: Vec<String>,
        ) -> Result<v2::EmbeddingsResult, v2::Error> {
            self.generate_embeddings(model, data).await
        }

        fn summary(&self) -> Option<String> {
            Some("local model".to_string())
        }
    }
}

/// The default engine creator for the LLM factor when used in the Spin CLI.
pub fn default_engine_creator(
    state_dir: Option<PathBuf>,
) -> anyhow::Result<impl LlmEngineCreator + 'static> {
    #[cfg(feature = "llm")]
    let engine = {
        use anyhow::Context as _;
        let models_dir_parent = match state_dir {
            Some(ref dir) => dir.clone(),
            None => std::env::current_dir().context("failed to get current working directory")?,
        };
        spin_llm_local::LocalLlmEngine::new(models_dir_parent.join("ai-models"))
    };
    #[cfg(not(feature = "llm"))]
    let engine = {
        let _ = state_dir;
        noop::NoopLlmEngine
    };
    let engine = Arc::new(Mutex::new(engine)) as Arc<Mutex<dyn LlmEngine>>;
    Ok(move || engine.clone())
}

#[async_trait]
impl LlmEngine for RemoteHttpLlmEngine {
    async fn infer(
        &mut self,
        model: v1::InferencingModel,
        prompt: String,
        params: v2::InferencingParams,
    ) -> Result<v2::InferencingResult, v2::Error> {
        spin_telemetry::monotonic_counter!(spin.llm_infer = 1, model_name = model);
        self.infer(model, prompt, params).await
    }

    async fn generate_embeddings(
        &mut self,
        model: v2::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<v2::EmbeddingsResult, v2::Error> {
        self.generate_embeddings(model, data).await
    }

    fn summary(&self) -> Option<String> {
        Some(format!("model at {}", self.url()))
    }
}

pub fn runtime_config_from_toml(
    table: &impl GetTomlValue,
    state_dir: Option<PathBuf>,
) -> anyhow::Result<Option<RuntimeConfig>> {
    let Some(value) = table.get("llm_compute") else {
        return Ok(None);
    };
    let config: LlmCompute = value.clone().try_into()?;

    Ok(Some(RuntimeConfig {
        engine: config.into_engine(state_dir)?,
    }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum LlmCompute {
    Spin,
    RemoteHttp(RemoteHttpCompute),
}

impl LlmCompute {
    fn into_engine(self, state_dir: Option<PathBuf>) -> anyhow::Result<Arc<Mutex<dyn LlmEngine>>> {
        let engine: Arc<Mutex<dyn LlmEngine>> = match self {
            #[cfg(not(feature = "llm"))]
            LlmCompute::Spin => {
                let _ = state_dir;
                Arc::new(Mutex::new(noop::NoopLlmEngine))
            }
            #[cfg(feature = "llm")]
            LlmCompute::Spin => default_engine_creator(state_dir)?.create(),
            LlmCompute::RemoteHttp(config) => Arc::new(Mutex::new(RemoteHttpLlmEngine::new(
                config.url,
                config.auth_token,
                config.custom_llm.and_then(|c| c.as_str().try_into().ok()),
            ))),
        };
        Ok(engine)
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct RemoteHttpCompute {
    url: Url,
    auth_token: String,
    custom_llm: Option<String>,
}

/// A noop engine used when the local engine feature is disabled.
#[cfg(not(feature = "llm"))]
mod noop {
    use super::*;

    #[derive(Clone, Copy)]
    pub(super) struct NoopLlmEngine;

    #[async_trait]
    impl LlmEngine for NoopLlmEngine {
        async fn infer(
            &mut self,
            _model: v2::InferencingModel,
            _prompt: String,
            _params: v2::InferencingParams,
        ) -> Result<v2::InferencingResult, v2::Error> {
            Err(v2::Error::RuntimeError(
                "Local LLM operations are not supported in this version of Spin.".into(),
            ))
        }

        async fn generate_embeddings(
            &mut self,
            _model: v2::EmbeddingModel,
            _data: Vec<String>,
        ) -> Result<v2::EmbeddingsResult, v2::Error> {
            Err(v2::Error::RuntimeError(
                "Local LLM operations are not supported in this version of Spin.".into(),
            ))
        }

        fn summary(&self) -> Option<String> {
            Some("noop model".to_owned())
        }
    }
}
