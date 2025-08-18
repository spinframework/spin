mod schemas;

use reqwest::{
    header::{HeaderMap, HeaderValue},
    Client, Url,
};
use spin_world::{
    async_trait,
    v2::llm::{self as wasi_llm},
};

use schemas::{
    CreateChatCompletionRequest, CreateChatCompletionResponseKind, CreateEmbeddingRequest,
    CreateEmbeddingResponseKind, Prompt, Role,
};

use crate::LlmWorker;

const CHAT_COMPLETIONS_ENDPOINT: &str = "/v1/chat/completions";
const EMBEDDINGS_ENDPOINT: &str = "/v1/embeddings";

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

        let url = self
            .url
            .join(CHAT_COMPLETIONS_ENDPOINT)
            .map_err(|_| wasi_llm::Error::RuntimeError("Failed to create URL".to_string()))?;

        tracing::info!("Sending remote inference request to {url}");

        let body = CreateChatCompletionRequest {
            // TODO: Make Role customizable
            messages: vec![Prompt::new(Role::User, prompt)],
            model,
            max_completion_tokens: Some(params.max_tokens),
            frequency_penalty: Some(params.repeat_penalty),
            reasoning_effort: None,
            verbosity: None,
        };

        let resp = client
            .request(reqwest::Method::POST, url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                wasi_llm::Error::RuntimeError(format!(
                    "POST {CHAT_COMPLETIONS_ENDPOINT} request error: {err}"
                ))
            })?;

        match resp.json::<CreateChatCompletionResponseKind>().await {
            Ok(CreateChatCompletionResponseKind::Success(val)) => Ok(val.into()),
            Ok(CreateChatCompletionResponseKind::Error { error }) => Err(error.into()),
            Err(err) => Err(wasi_llm::Error::RuntimeError(format!(
                "Failed to deserialize response for \"POST  {CHAT_COMPLETIONS_ENDPOINT}\": {err}"
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

        let body = CreateEmbeddingRequest {
            input: data,
            model,
            encoding_format: None,
            dimensions: None,
            user: None,
        };

        let url = self
            .url
            .join(EMBEDDINGS_ENDPOINT)
            .map_err(|_| wasi_llm::Error::RuntimeError("Failed to create URL".to_string()))?;

        tracing::info!("Sending remote embedding request to {url}");

        let resp = client
            .request(reqwest::Method::POST, url)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                wasi_llm::Error::RuntimeError(format!(
                    "POST {EMBEDDINGS_ENDPOINT} request error: {err}"
                ))
            })?;

        match resp.json::<CreateEmbeddingResponseKind>().await {
            Ok(CreateEmbeddingResponseKind::Success(val)) => Ok(val.into()),
            Ok(CreateEmbeddingResponseKind::Error { error }) => Err(error.into()),
            Err(err) => Err(wasi_llm::Error::RuntimeError(format!(
                "Failed to deserialize response  for \"POST  {EMBEDDINGS_ENDPOINT}\": {err}"
            ))),
        }
    }

    fn url(&self) -> Url {
        self.url.clone()
    }
}
