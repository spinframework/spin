use anyhow::Result;
use reqwest::Url;
use spin_world::{
    async_trait,
    v2::llm::{self as wasi_llm},
};

mod default;
mod open_ai;

pub struct RemoteHttpLlmEngine {
    worker: Box<dyn LlmWorker>,
}

impl RemoteHttpLlmEngine {
    pub fn new(url: Url, auth_token: String, api_type: ApiType) -> Self {
        let worker: Box<dyn LlmWorker> = match api_type {
            ApiType::OpenAi => Box::new(open_ai::AgentEngine::new(auth_token, url, None)),
            ApiType::Default => Box::new(default::AgentEngine::new(auth_token, url, None)),
        };
        Self { worker }
    }
}

#[async_trait]
pub trait LlmWorker: Send + Sync {
    async fn infer(
        &mut self,
        model: wasi_llm::InferencingModel,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error>;

    async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error>;

    fn url(&self) -> Url;
}

impl RemoteHttpLlmEngine {
    pub async fn infer(
        &mut self,
        model: wasi_llm::InferencingModel,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error> {
        self.worker.infer(model, prompt, params).await
    }

    pub async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error> {
        self.worker.generate_embeddings(model, data).await
    }

    pub fn url(&self) -> Url {
        self.worker.url()
    }
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    /// Compatible with OpenAI's API alongside some other LLMs
    OpenAi,
    #[default]
    Default,
}
