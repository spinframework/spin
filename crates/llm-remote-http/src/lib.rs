use anyhow::Result;
use futures::stream::TryStreamExt as _;
use reqwest::Url;
use spin_world::{
    async_trait,
    v2::llm::{self as wasi_llm},
};

mod default;
mod open_ai;

async fn read_body(
    resp: reqwest::Response,
    max_result_bytes: usize,
) -> Result<Vec<u8>, wasi_llm::Error> {
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream
        .try_next()
        .await
        .map_err(|err| wasi_llm::Error::RuntimeError(format!("POST /infer request error: {err}")))?
    {
        body.extend(chunk);
        if body.len() > max_result_bytes {
            return Err(wasi_llm::Error::RuntimeError(format!(
                "query result exceeds limit of {max_result_bytes} bytes"
            )));
        }
    }
    Ok(body)
}

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
        max_result_bytes: usize,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error>;

    async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
        max_result_bytes: usize,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error>;

    fn url(&self) -> Url;
}

impl RemoteHttpLlmEngine {
    pub async fn infer(
        &mut self,
        model: wasi_llm::InferencingModel,
        prompt: String,
        params: wasi_llm::InferencingParams,
        max_result_bytes: usize,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error> {
        self.worker
            .infer(model, prompt, params, max_result_bytes)
            .await
    }

    pub async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
        max_result_bytes: usize,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error> {
        self.worker
            .generate_embeddings(model, data, max_result_bytes)
            .await
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
