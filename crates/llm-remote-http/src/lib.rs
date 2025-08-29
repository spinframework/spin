use anyhow::Result;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use spin_world::{
    async_trait,
    v2::llm::{self as wasi_llm},
};

use crate::schema::{ChatCompletionChoice, Embedding};

mod default;
mod open_ai;
mod schema;

pub struct RemoteHttpLlmEngine {
    worker: Box<dyn LlmWorker>,
}

impl RemoteHttpLlmEngine {
    pub fn new(url: Url, auth_token: String, custom_llm: CustomLlm) -> Self {
        let worker: Box<dyn LlmWorker> = match custom_llm {
            CustomLlm::OpenAi => Box::new(open_ai::OpenAIAgentEngine::new(auth_token, url, None)),
            CustomLlm::Default => Box::new(default::DefaultAgentEngine::new(auth_token, url, None)),
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

#[derive(Serialize)]
#[serde(rename_all(serialize = "camelCase"))]
struct InferRequestBodyParams {
    max_tokens: u32,
    repeat_penalty: f32,
    repeat_penalty_last_n_token_count: u32,
    temperature: f32,
    top_k: u32,
    top_p: f32,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
struct InferUsage {
    prompt_token_count: u32,
    generated_token_count: u32,
}

#[derive(Deserialize)]
struct InferResponseBody {
    text: String,
    usage: InferUsage,
}

#[derive(Deserialize)]
struct CreateChatCompletionResponse {
    /// A unique identifier for the chat completion.
    #[serde(rename = "id")]
    _id: String,
    /// The object type, which is always `chat.completion`.
    #[serde(rename = "object")]
    _object: String,
    /// The Unix timestamp (in seconds) of when the chat completion was created.
    #[serde(rename = "created")]
    _created: u64,
    /// The model used for the chat completion.
    #[serde(rename = "model")]
    _model: String,
    /// This fingerprint represents the backend configuration that the model runs with.
    ///
    /// While it's deprecated, it's still provided for compatibility with older clients.
    #[serde(rename = "system_fingerprint")]
    _system_fingerprint: Option<String>,
    /// A list of chat completion choices. Can be more than one if `n` is greater than 1.
    choices: Vec<ChatCompletionChoice>,
    /// Usage statistics for the completion request
    #[serde(rename = "usage")]
    usage: CompletionUsage,
}

#[derive(Deserialize)]
struct CompletionUsage {
    /// Number of tokens in the generated completion.
    completion_tokens: u32,
    /// Number of tokens in the prompt.
    prompt_tokens: u32,
    /// Total number of tokens used in the request (prompt + completion).
    #[serde(rename = "total_tokens")]
    _total_tokens: u32,
}

#[derive(Deserialize)]
#[serde(rename_all(deserialize = "camelCase"))]
struct EmbeddingUsage {
    prompt_token_count: u32,
}

#[derive(Deserialize)]
struct EmbeddingResponseBody {
    embeddings: Vec<Vec<f32>>,
    usage: EmbeddingUsage,
}

#[derive(Deserialize)]
struct CreateEmbeddingResponse {
    #[serde(rename = "object")]
    _object: String,
    #[serde(rename = "model")]
    _model: String,
    data: Vec<Embedding>,
    usage: OpenAIEmbeddingUsage,
}

impl CreateEmbeddingResponse {
    fn embeddings(&self) -> Vec<Vec<f32>> {
        self.data
            .iter()
            .map(|embedding| embedding.embedding.clone())
            .collect()
    }
}

#[derive(Deserialize)]
struct OpenAIEmbeddingUsage {
    prompt_tokens: u32,
    #[serde(rename = "total_tokens")]
    _total_tokens: u32,
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

impl From<InferResponseBody> for wasi_llm::InferencingResult {
    fn from(value: InferResponseBody) -> Self {
        Self {
            text: value.text,
            usage: wasi_llm::InferencingUsage {
                prompt_token_count: value.usage.prompt_token_count,
                generated_token_count: value.usage.generated_token_count,
            },
        }
    }
}

impl From<CreateChatCompletionResponse> for wasi_llm::InferencingResult {
    fn from(value: CreateChatCompletionResponse) -> Self {
        Self {
            text: value.choices[0].message.content.clone(),
            usage: wasi_llm::InferencingUsage {
                prompt_token_count: value.usage.prompt_tokens,
                generated_token_count: value.usage.completion_tokens,
            },
        }
    }
}

impl From<EmbeddingResponseBody> for wasi_llm::EmbeddingsResult {
    fn from(value: EmbeddingResponseBody) -> Self {
        Self {
            embeddings: value.embeddings,
            usage: wasi_llm::EmbeddingsUsage {
                prompt_token_count: value.usage.prompt_token_count,
            },
        }
    }
}

impl From<CreateEmbeddingResponse> for wasi_llm::EmbeddingsResult {
    fn from(value: CreateEmbeddingResponse) -> Self {
        Self {
            embeddings: value.embeddings(),
            usage: wasi_llm::EmbeddingsUsage {
                prompt_token_count: value.usage.prompt_tokens,
            },
        }
    }
}

#[derive(Debug, Default, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CustomLlm {
    /// Compatible with OpenAI's API alongside some other LLMs
    OpenAi,
    #[default]
    Default,
}
