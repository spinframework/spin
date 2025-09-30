use serde::{Deserialize, Serialize};
use spin_world::v2::llm as wasi_llm;

#[derive(Serialize, Debug)]
pub struct CreateChatCompletionRequest {
    pub messages: Vec<Prompt>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct CreateEmbeddingRequest {
    pub input: Vec<String>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<EncodingFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum CreateChatCompletionResponseKind {
    Success(CreateChatCompletionResponse),
    Error { error: ResponseError },
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum CreateEmbeddingResponseKind {
    Success(CreateEmbeddingResponse),
    Error { error: ResponseError },
}

#[derive(Deserialize)]
pub struct CreateChatCompletionResponse {
    /// A list of chat completion choices. Can be more than one if `n` is greater than 1.
    choices: Vec<ChatCompletionChoice>,
    /// Usage statistics for the completion request
    usage: CompletionUsage,
}

#[derive(Deserialize)]
struct CompletionUsage {
    /// Number of tokens in the generated completion.
    completion_tokens: u32,
    /// Number of tokens in the prompt.
    prompt_tokens: u32,
}

#[derive(Deserialize)]
pub struct CreateEmbeddingResponse {
    data: Vec<Embedding>,
    usage: EmbeddingUsage,
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
struct EmbeddingUsage {
    prompt_tokens: u32,
}

impl From<CreateChatCompletionResponse> for wasi_llm::InferencingResult {
    fn from(value: CreateChatCompletionResponse) -> Self {
        Self {
            text: value
                .choices
                .first()
                .map_or_else(String::new, |c| c.message.content.clone()),
            usage: wasi_llm::InferencingUsage {
                prompt_token_count: value.usage.prompt_tokens,
                generated_token_count: value.usage.completion_tokens,
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

#[derive(Serialize, Debug)]
pub(crate) struct Prompt {
    role: Role,
    content: String,
}

impl Prompt {
    pub fn new(role: Role, content: String) -> Self {
        Self { role, content }
    }
}

#[derive(Serialize, Debug)]
pub(crate) enum Role {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "tool")]
    Tool,
}

impl TryFrom<&str> for Role {
    type Error = wasi_llm::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "system" => Ok(Role::System),
            "user" => Ok(Role::User),
            "assistant" => Ok(Role::Assistant),
            "tool" => Ok(Role::Tool),
            _ => Err(wasi_llm::Error::InvalidInput(format!(
                "{value} not a valid role"
            ))),
        }
    }
}

#[derive(Serialize, Debug)]
pub(crate) enum EncodingFormat {
    #[serde(rename = "float")]
    Float,
    #[serde(rename = "base64")]
    Base64,
}

impl TryFrom<&str> for EncodingFormat {
    type Error = wasi_llm::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "float" => Ok(EncodingFormat::Float),
            "base64" => Ok(EncodingFormat::Base64),
            _ => Err(wasi_llm::Error::InvalidInput(format!(
                "{value} not a valid encoding format"
            ))),
        }
    }
}

#[derive(Serialize, Debug)]
enum ReasoningEffort {
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
}

impl TryFrom<&str> for ReasoningEffort {
    type Error = wasi_llm::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "minimal" => Ok(ReasoningEffort::Minimal),
            "low" => Ok(ReasoningEffort::Low),
            "medium" => Ok(ReasoningEffort::Medium),
            "high" => Ok(ReasoningEffort::High),
            _ => Err(wasi_llm::Error::InvalidInput(format!(
                "{value} not a recognized reasoning effort",
            ))),
        }
    }
}

#[derive(Serialize, Debug)]
enum Verbosity {
    Low,
    Medium,
    High,
}

impl TryFrom<&str> for Verbosity {
    type Error = wasi_llm::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "low" => Ok(Verbosity::Low),
            "medium" => Ok(Verbosity::Medium),
            "high" => Ok(Verbosity::High),
            _ => Err(wasi_llm::Error::InvalidInput(format!(
                "{value} not a recognized verbosity",
            ))),
        }
    }
}

#[derive(Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionResponseMessage,
}

#[derive(Deserialize)]
/// A chat completion message generated by the model.
struct ChatCompletionResponseMessage {
    /// The contents of the message
    content: String,
}

#[derive(Deserialize)]
struct Embedding {
    /// The embedding vector, which is a list of floats. The length of vector depends on the model as
    /// listed in the [embedding guide](https://platform.openai.com/docs/guides/embeddings).
    embedding: Vec<f32>,
}

#[derive(Deserialize, Default)]
pub(crate) struct ResponseError {
    message: String,
}

impl From<ResponseError> for wasi_llm::Error {
    fn from(value: ResponseError) -> Self {
        wasi_llm::Error::RuntimeError(value.message)
    }
}
