use std::fmt::Display;

use serde::{Deserialize, Serialize};
use spin_world::v2::llm as wasi_llm;

/// LLM model
#[derive(Serialize, Debug)]
pub enum Model {
    GPT5,
    GPT5Mini,
    GPT5Nano,
    GPT5Chat,
    GPT45,
    GPT41,
    GPT41Mini,
    GPT41Nano,
    GPT4,
    GPT4o,
    GPT4oMini,
    O4Mini,
    O3,
    O1,
}

impl TryFrom<&str> for Model {
    type Error = wasi_llm::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "gpt-5" => Ok(Model::GPT5),
            "gpt-5-mini" => Ok(Model::GPT5Mini),
            "gpt-5-nano" => Ok(Model::GPT5Nano),
            "gpt-5-chat" => Ok(Model::GPT5Chat),
            "gpt-4.5" => Ok(Model::GPT45),
            "gpt-4.1" => Ok(Model::GPT41),
            "gpt-4.1-mini" => Ok(Model::GPT41Mini),
            "gpt-4.1-nano" => Ok(Model::GPT41Nano),
            "gpt-4" => Ok(Model::GPT4),
            "gpt-4o" => Ok(Model::GPT4o),
            "gpt-4o-mini" => Ok(Model::GPT4oMini),
            "o4-mini" => Ok(Model::O4Mini),
            "o3" => Ok(Model::O3),
            "o1" => Ok(Model::O1),
            _ => Err(wasi_llm::Error::InvalidInput(format!(
                "{value} is not a valid model name" // TODO: Joshua: Have some public docs to state the supported models to point users to
            ))),
        }
    }
}

impl Display for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Model::GPT5 => write!(f, "gpt-5"),
            Model::GPT5Mini => write!(f, "gpt-5-mini"),
            Model::GPT5Nano => write!(f, "gpt-5-nano"),
            Model::GPT5Chat => write!(f, "gpt-5-chat"),
            Model::GPT45 => write!(f, "gpt-4.5"),
            Model::GPT41 => write!(f, "gpt-4.1"),
            Model::GPT41Mini => write!(f, "gpt-4.1-mini"),
            Model::GPT41Nano => write!(f, "gpt-4.1-nano"),
            Model::GPT4 => write!(f, "gpt-4"),
            Model::GPT4o => write!(f, "gpt-4o"),
            Model::GPT4oMini => write!(f, "gpt-4o-mini"),
            Model::O4Mini => write!(f, "o4-mini"),
            Model::O3 => write!(f, "o3"),
            Model::O1 => write!(f, "o1"),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct Prompt {
    role: Role,
    content: String,
}

impl Prompt {
    pub fn new(role: Role, content: String) -> Self {
        Self { role, content }
    }
}

#[derive(Serialize, Debug)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::System => write!(f, "system"),
            Role::User => write!(f, "user"),
            Role::Assistant => write!(f, "assistant"),
            Role::Tool => write!(f, "tool"),
        }
    }
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
pub enum EncodingFormat {
    Float,
    Base64,
}

impl Display for EncodingFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodingFormat::Float => write!(f, "float"),
            EncodingFormat::Base64 => write!(f, "base64"),
        }
    }
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
pub enum EmbeddingModels {
    TextEmbeddingAda002,
    TextEmbedding3Small,
    TextEmbedding3Large,
    Custom(String),
}

impl Display for EmbeddingModels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbeddingModels::TextEmbeddingAda002 => write!(f, "text-embedding-ada-002"),
            EmbeddingModels::TextEmbedding3Small => write!(f, "text-embedding-3-small"),
            EmbeddingModels::TextEmbedding3Large => write!(f, "text-embedding-3-large"),
            EmbeddingModels::Custom(model) => write!(f, "{model}"),
        }
    }
}

impl TryFrom<&str> for EmbeddingModels {
    type Error = wasi_llm::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "text-embedding-ada-002" => Ok(EmbeddingModels::TextEmbeddingAda002),
            "text-embedding-3-small" => Ok(EmbeddingModels::TextEmbedding3Small),
            "text-embedding-3-large" => Ok(EmbeddingModels::TextEmbedding3Large),
            _ => Ok(EmbeddingModels::Custom(value.to_string())),
        }
    }
}

#[derive(Serialize, Debug)]
enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
}

impl Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningEffort::Minimal => write!(f, "minimal"),
            ReasoningEffort::Low => write!(f, "low"),
            ReasoningEffort::Medium => write!(f, "medium"),
            ReasoningEffort::High => write!(f, "high"),
        }
    }
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
pub struct ChatCompletionChoice {
    /// The index of the choice in the list of choices
    _index: u32,
    pub message: ChatCompletionResponseMessage,
    /// The reason the model stopped generating tokens. This will be `stop` if the model hit a
    /// natural stop point or a provided stop sequence,
    _finish_reason: String,
    /// Log probability information for the choice.
    _logprobs: Option<Logprobs>,
}

#[derive(Deserialize)]
/// A chat completion message generated by the model.
pub struct ChatCompletionResponseMessage {
    /// The role of the author of this message
    _role: String,
    /// The contents of the message
    pub content: String,
    /// The refusal message generated by the model
    _refusal: Option<String>,
}

#[derive(Deserialize)]
pub struct Logprobs {
    /// A list of message content tokens with log probability information.
    _content: Option<Vec<String>>,
    /// A list of message refusal tokens with log probability information.
    _refusal: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct Embedding {
    /// The index of the embedding in the list of embeddings..
    _index: u32,
    /// The embedding vector, which is a list of floats. The length of vector depends on the model as
    /// listed in the [embedding guide](https://platform.openai.com/docs/guides/embeddings).
    pub embedding: Vec<f32>,
    /// The object type, which is always "embedding"
    _object: String,
}
