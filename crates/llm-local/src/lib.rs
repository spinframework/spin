mod bert;
mod llama;

use anyhow::Context;
use bert::{BertModel, Config};
use candle::{safetensors::load_buffer, DType};
use candle_nn::VarBuilder;
use spin_common::ui::quoted_path;
use spin_core::async_trait;
use spin_world::v2::llm::{self as wasi_llm};
use std::{
    collections::{hash_map::Entry, HashMap},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokenizers::PaddingParams;

const MODEL_ALL_MINILM_L6_V2: &str = "all-minilm-l6-v2";
type ModelName = String;

#[derive(Clone)]
pub struct LocalLlmEngine {
    registry: PathBuf,
    inferencing_models: HashMap<ModelName, Arc<dyn InferencingModel>>,
    embeddings_models: HashMap<String, Arc<(tokenizers::Tokenizer, BertModel)>>,
}

#[derive(Debug)]
enum InferencingModelArch {
    Llama,
}

impl FromStr for InferencingModelArch {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "llama" => Ok(InferencingModelArch::Llama),
            _ => Err(()),
        }
    }
}

/// A model that is prepared and cached after loading.
///
/// This trait does not specify anything about if the results are cached.
#[async_trait]
trait InferencingModel: Send + Sync {
    async fn infer(
        &self,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> anyhow::Result<wasi_llm::InferencingResult>;
}

impl LocalLlmEngine {
    pub async fn infer(
        &mut self,
        model: wasi_llm::InferencingModel,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> Result<wasi_llm::InferencingResult, wasi_llm::Error> {
        let model = self.inferencing_model(model).await?;

        model
            .infer(prompt, params)
            .await
            .map_err(|e| wasi_llm::Error::RuntimeError(e.to_string()))
    }

    pub async fn generate_embeddings(
        &mut self,
        model: wasi_llm::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<wasi_llm::EmbeddingsResult, wasi_llm::Error> {
        let model = self.embeddings_model(model).await?;
        generate_embeddings(data, model).await.map_err(|e| {
            wasi_llm::Error::RuntimeError(format!("Error occurred generating embeddings: {e}"))
        })
    }
}

impl LocalLlmEngine {
    pub fn new(registry: PathBuf) -> Self {
        Self {
            registry,
            inferencing_models: Default::default(),
            embeddings_models: Default::default(),
        }
    }

    /// Get embeddings model from cache or load from disk
    async fn embeddings_model(
        &mut self,
        model: wasi_llm::EmbeddingModel,
    ) -> Result<Arc<(tokenizers::Tokenizer, BertModel)>, wasi_llm::Error> {
        let key = match model.as_str() {
            MODEL_ALL_MINILM_L6_V2 => model,
            _ => return Err(wasi_llm::Error::ModelNotSupported),
        };
        let registry_path = self.registry.join(&key);
        let r = match self.embeddings_models.entry(key) {
            Entry::Occupied(o) => o.get().clone(),
            Entry::Vacant(v) => v
                .insert({
                    tokio::task::spawn_blocking(move || {
                        if !registry_path.exists() {
                            return Err(
                                wasi_llm::Error::RuntimeError(format!(
                                "The directory expected to house the embeddings models '{}' does not exist.",
                                registry_path.display()
                            )));
                        }
                        let tokenizer_file = registry_path.join("tokenizer.json");
                        let model_file = registry_path.join("model.safetensors");
                        let tokenizer = load_tokenizer(&tokenizer_file).map_err(|_| {
                            wasi_llm::Error::RuntimeError(format!(
                                "Failed to load embeddings tokenizer from '{}'",
                                tokenizer_file.display()
                            ))
                        })?;
                        let model = load_model(&model_file).map_err(|_| {
                            wasi_llm::Error::RuntimeError(format!(
                                "Failed to load embeddings model from '{}'",
                                model_file.display()
                            ))
                        })?;
                        Ok(Arc::new((tokenizer, model)))
                    })
                    .await
                    .map_err(|_| {
                        wasi_llm::Error::RuntimeError("Error loading inferencing model".into())
                    })??
                })
                .clone(),
        };
        Ok(r)
    }

    /// Get inferencing model from cache or load from disk
    async fn inferencing_model(
        &mut self,
        model: wasi_llm::InferencingModel,
    ) -> Result<Arc<dyn InferencingModel>, wasi_llm::Error> {
        let model = match self.inferencing_models.entry(model.clone()) {
            Entry::Occupied(o) => o.get().clone(),
            Entry::Vacant(v) => {
                let (model_dir, arch) =
                    walk_registry_for_model(&self.registry, model.clone()).await?;
                let model: Arc<dyn InferencingModel> = match arch {
                    InferencingModelArch::Llama => Arc::new(
                        llama::LlamaModels::new(&model_dir)
                            .await
                            .map_err(|e| wasi_llm::Error::RuntimeError(e.to_string()))?,
                    ),
                };

                v.insert(model.clone());

                model
            }
        };
        Ok(model)
    }
}

/// Walks the registry file structure and returns the directory the model is
/// present along with its architecture
async fn walk_registry_for_model(
    registry_path: &Path,
    model: String,
) -> Result<(PathBuf, InferencingModelArch), wasi_llm::Error> {
    let mut arch_dirs = tokio::fs::read_dir(registry_path).await.map_err(|e| {
        wasi_llm::Error::RuntimeError(format!(
            "Could not read model registry directory '{}': {e}",
            registry_path.display()
        ))
    })?;
    let mut result = None;
    'outer: while let Some(arch_dir) = arch_dirs.next_entry().await.map_err(|e| {
        wasi_llm::Error::RuntimeError(format!(
            "Failed to read arch directory in model registry: {e}"
        ))
    })? {
        if arch_dir
            .file_type()
            .await
            .map_err(|e| {
                wasi_llm::Error::RuntimeError(format!(
                    "Could not read file type of '{}' dir: {e}",
                    arch_dir.path().display()
                ))
            })?
            .is_file()
        {
            continue;
        }
        let mut model_dirs = tokio::fs::read_dir(arch_dir.path()).await.map_err(|e| {
            wasi_llm::Error::RuntimeError(format!(
                "Error reading architecture directory in model registry: {e}"
            ))
        })?;
        while let Some(model_dir) = model_dirs.next_entry().await.map_err(|e| {
            wasi_llm::Error::RuntimeError(format!(
                "Error reading model folder in model registry: {e}"
            ))
        })? {
            // Models need to be a directory. So ignore any files.
            if model_dir
                .file_type()
                .await
                .map_err(|e| {
                    wasi_llm::Error::RuntimeError(format!(
                        "Could not read file type of '{}' dir: {e}",
                        model_dir.path().display()
                    ))
                })?
                .is_file()
            {
                continue;
            }
            if model_dir
                .file_name()
                .to_str()
                .map(|m| m == model)
                .unwrap_or_default()
            {
                let arch = arch_dir.file_name();
                let arch = arch
                    .to_str()
                    .ok_or(wasi_llm::Error::ModelNotSupported)?
                    .parse()
                    .map_err(|_| wasi_llm::Error::ModelNotSupported)?;
                result = Some((model_dir.path(), arch));
                break 'outer;
            }
        }
    }

    result.ok_or_else(|| {
        wasi_llm::Error::InvalidInput(format!(
            "no model directory found in registry for model '{model}'"
        ))
    })
}

async fn generate_embeddings(
    data: Vec<String>,
    model: Arc<(tokenizers::Tokenizer, BertModel)>,
) -> anyhow::Result<wasi_llm::EmbeddingsResult> {
    let n_sentences = data.len();
    tokio::task::spawn_blocking(move || {
        let mut tokenizer = model.0.clone();
        let model = &model.1;
        // This function attempts to generate the embeddings for a batch of inputs, most
        // likely of different lengths.
        // The tokenizer expects all inputs in a batch to have the same length, so the
        // following is configuring the tokenizer to pad (add trailing zeros) each input
        // to match the length of the longest in the batch.
        if let Some(pp) = tokenizer.get_padding_mut() {
            pp.strategy = tokenizers::PaddingStrategy::BatchLongest
        } else {
            let pp = PaddingParams {
                strategy: tokenizers::PaddingStrategy::BatchLongest,
                ..Default::default()
            };
            tokenizer.with_padding(Some(pp));
        }
        let tokens = tokenizer
            .encode_batch(data, true)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let token_ids = tokens
            .iter()
            .map(|tokens| {
                let tokens = tokens.get_ids().to_vec();
                Ok(candle::Tensor::new(
                    tokens.as_slice(),
                    &candle::Device::Cpu,
                )?)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        // Execute the model's forward propagation function, which generates the raw embeddings.
        let token_ids = candle::Tensor::stack(&token_ids, 0)?;
        let embeddings = model.forward(&token_ids, &token_ids.zeros_like()?)?;

        // SBERT adds a pooling operation to the raw output to derive a fixed sized sentence embedding.
        // The BERT models suggest using mean pooling, which is what the operation below performs.
        // https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2#usage-huggingface-transformers
        let (_, n_tokens, _) = embeddings.dims3()?;
        let embeddings = (embeddings.sum(1)? / (n_tokens as f64))?;

        // Take each sentence embedding from the batch and arrange it in the final result tensor.
        // Normalize each embedding as the last step (this generates vectors with length 1, which
        // makes the cosine similarity function significantly more efficient (it becomes a simple
        // dot product).
        let mut results: Vec<Vec<f32>> = Vec::new();
        for j in 0..n_sentences {
            let e_j = embeddings.get(j)?;
            let mut emb: Vec<f32> = e_j.to_vec1()?;
            let length: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
            emb.iter_mut().for_each(|x| *x /= length);
            results.push(emb);
        }

        let result = wasi_llm::EmbeddingsResult {
            embeddings: results,
            usage: wasi_llm::EmbeddingsUsage {
                prompt_token_count: n_tokens as u32,
            },
        };
        Ok(result)
    })
    .await?
}

fn load_tokenizer(tokenizer_file: &Path) -> anyhow::Result<tokenizers::Tokenizer> {
    let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_file).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read tokenizer file {}: {e}",
            quoted_path(tokenizer_file)
        )
    })?;
    Ok(tokenizer)
}

fn load_model(model_file: &Path) -> anyhow::Result<BertModel> {
    let device = &candle::Device::Cpu;
    let data = std::fs::read(model_file)?;
    let tensors = load_buffer(&data, device)?;
    let vb = VarBuilder::from_tensors(tensors, DType::F32, device);
    let model = BertModel::load(vb, &Config::default()).context("error loading bert model")?;
    Ok(model)
}
