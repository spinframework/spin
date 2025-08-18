use anyhow::Result;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client, Url,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use spin_world::{
    async_trait,
    v2::llm::{self as wasi_llm},
};

use crate::LlmWorker;

pub(crate) struct AgentEngine {
    auth_token: String,
    url: Url,
    client: Option<Client>,
}

impl AgentEngine {
    pub fn new(auth_token: String, url: Url, client: Option<Client>) -> Self {
        Self {
            auth_token,
            url,
            client,
        }
    }
}

#[async_trait]
impl LlmWorker for AgentEngine {
    async fn infer(
        &mut self,
        model: wasi_llm::InferencingModel,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error> {
        let client = self.client.get_or_insert_with(Default::default);

        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("bearer {}", self.auth_token)).map_err(|_| {
                wasi_llm::Error::RuntimeError("Failed to create authorization header".to_string())
            })?,
        );
        spin_telemetry::inject_trace_context(&mut headers);

        let inference_options = InferRequestBodyParams {
            max_tokens: params.max_tokens,
            repeat_penalty: params.repeat_penalty,
            repeat_penalty_last_n_token_count: params.repeat_penalty_last_n_token_count,
            temperature: params.temperature,
            top_k: params.top_k,
            top_p: params.top_p,
        };
        let body = serde_json::to_string(&json!({
            "model": model,
            "prompt": prompt,
            "options": inference_options
        }))
        .map_err(|_| wasi_llm::Error::RuntimeError("Failed to serialize JSON".to_string()))?;

        let infer_url = self
            .url
            .join("/infer")
            .map_err(|_| wasi_llm::Error::RuntimeError("Failed to create URL".to_string()))?;
        tracing::info!("Sending remote inference request to {infer_url}");

        let resp = client
            .request(reqwest::Method::POST, infer_url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|err| {
                wasi_llm::Error::RuntimeError(format!("POST /infer request error: {err}"))
            })?;

        match resp.json::<InferResponseBody>().await {
            Ok(val) => Ok(val.into()),
            Err(err) => Err(wasi_llm::Error::RuntimeError(format!(
                "Failed to deserialize response for \"POST  /index\": {err}"
            ))),
        }
    }

    async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error> {
        let client = self.client.get_or_insert_with(Default::default);

        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("bearer {}", self.auth_token)).map_err(|_| {
                wasi_llm::Error::RuntimeError("Failed to create authorization header".to_string())
            })?,
        );
        spin_telemetry::inject_trace_context(&mut headers);

        let body = serde_json::to_string(&json!({
            "model": model,
            "input": data
        }))
        .map_err(|_| wasi_llm::Error::RuntimeError("Failed to serialize JSON".to_string()))?;

        let resp = client
            .request(
                reqwest::Method::POST,
                self.url.join("/embed").map_err(|_| {
                    wasi_llm::Error::RuntimeError("Failed to create URL".to_string())
                })?,
            )
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|err| {
                wasi_llm::Error::RuntimeError(format!("POST /embed request error: {err}"))
            })?;

        match resp.json::<EmbeddingResponseBody>().await {
            Ok(val) => Ok(val.into()),
            Err(err) => Err(wasi_llm::Error::RuntimeError(format!(
                "Failed to deserialize response  for \"POST  /embed\": {err}"
            ))),
        }
    }

    fn url(&self) -> Url {
        self.url.clone()
    }
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
pub struct InferUsage {
    prompt_token_count: u32,
    generated_token_count: u32,
}

#[derive(Deserialize)]
pub struct InferResponseBody {
    text: String,
    usage: InferUsage,
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
