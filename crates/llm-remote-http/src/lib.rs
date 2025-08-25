use anyhow::Result;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use spin_world::v2::llm::{self as wasi_llm};

use crate::{
    default::DefaultAgentEngine,
    open_ai::OpenAIAgentEngine,
    schema::{ChatCompletionChoice, Embedding},
};

mod default;
mod open_ai;
mod schema;

#[derive(Clone)]
pub enum Agent {
    //TODO: Joshua: Naming??!
    Default {
        auth_token: String,
        url: Url,
        client: Option<Client>,
    },
    OpenAI {
        auth_token: String,
        url: Url,
        client: Option<Client>,
    },
}

impl Agent {
    pub fn from(url: Url, auth_token: String, agent: Option<CustomLlm>) -> Self {
        match agent {
            Some(CustomLlm::OpenAi) => Agent::OpenAI {
                auth_token,
                url,
                client: None,
            },
            _ => Agent::Default {
                auth_token,
                url,
                client: None,
            },
        }
    }
}

#[derive(Clone)]
pub struct RemoteHttpLlmEngine {
    agent: Agent,
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
    _id: String,
    _object: String,
    _created: u64,
    _model: String,
    choices: Vec<ChatCompletionChoice>,
    usage: CompletionUsage,
}

#[derive(Deserialize)]
struct CompletionUsage {
    completion_tokens: u32,
    prompt_tokens: u32,
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
    _object: String,
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
    _total_tokens: u32,
}

impl RemoteHttpLlmEngine {
    pub fn new(url: Url, auth_token: String, agent: Option<CustomLlm>) -> Self {
        RemoteHttpLlmEngine {
            agent: Agent::from(url, auth_token, agent),
        }
    }

    pub async fn infer(
        &mut self,
        model: wasi_llm::InferencingModel,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error> {
        match &self.agent {
            Agent::OpenAI {
                auth_token,
                url,
                client,
            } => {
                OpenAIAgentEngine::infer(auth_token, url, client.clone(), model, prompt, params)
                    .await
            }
            Agent::Default {
                auth_token,
                url,
                client,
            } => {
                DefaultAgentEngine::infer(auth_token, url, client.clone(), model, prompt, params)
                    .await
            }
        }
    }

    pub async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error> {
        match &self.agent {
            Agent::OpenAI {
                auth_token,
                url,
                client,
            } => {
                OpenAIAgentEngine::generate_embeddings(auth_token, url, client.clone(), model, data)
                    .await
            }
            Agent::Default {
                auth_token,
                url,
                client,
            } => {
                DefaultAgentEngine::generate_embeddings(
                    auth_token,
                    url,
                    client.clone(),
                    model,
                    data,
                )
                .await
            }
        }
    }

    pub fn url(&self) -> Url {
        match &self.agent {
            Agent::OpenAI { url, .. } => url.clone(),
            Agent::Default { url, .. } => url.clone(),
        }
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

#[derive(Debug, serde::Deserialize, PartialEq)]
pub enum CustomLlm {
    OpenAi,
}

impl TryFrom<&str> for CustomLlm {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_str() {
            "open_ai" | "openai" => Ok(CustomLlm::OpenAi),
            _ => Err(anyhow::anyhow!("Invalid custom LLM: {}", value)),
        }
    }
}
