use anyhow::Result;
use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client, Url,
};
use serde_json::json;
use spin_world::{
    async_trait,
    v2::llm::{self as wasi_llm},
};

use crate::{EmbeddingResponseBody, InferRequestBodyParams, InferResponseBody, LlmWorker};

pub(crate) struct DefaultAgentEngine {
    auth_token: String,
    url: Url,
    client: Option<Client>,
}

impl DefaultAgentEngine {
    pub fn new(auth_token: String, url: Url, client: Option<Client>) -> Self {
        Self {
            auth_token,
            url,
            client,
        }
    }
}

#[async_trait]
impl LlmWorker for DefaultAgentEngine {
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
