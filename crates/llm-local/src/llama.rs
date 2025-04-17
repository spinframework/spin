use crate::InferencingModel;
use anyhow::{anyhow, bail, Context, Result};
use candle::{safetensors::load_buffer, utils, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::{
    generation::{LogitsProcessor, Sampling},
    models::llama::{self, Cache, Config, Llama, LlamaConfig},
};
use rand::{RngCore, SeedableRng};
use spin_core::async_trait;
use spin_world::v2::llm::{self as wasi_llm, InferencingUsage};
use std::{collections::HashMap, fs, path::Path, sync::Arc};
use tokenizers::Tokenizer;

const TOKENIZER_FILENAME: &str = "tokenizer.json";
const CONFIG_FILENAME: &str = "config.json";
const EOS_TOKEN: &str = "</s>";
const MODEL_SAFETENSORS_INDEX_FILE: &str = "model.safetensors.index.json";

pub fn auto_device() -> Result<Device> {
    if utils::cuda_is_available() {
        Ok(Device::new_cuda(0)?)
    } else if utils::metal_is_available() {
        Ok(Device::new_metal(0)?)
    } else {
        Ok(Device::Cpu)
    }
}

#[derive(Clone)]
pub(crate) struct LlamaModels {
    model: Arc<Llama>,
    config: Config,
    cache: Cache,
    tokenizer: Tokenizer,
    device: Device,
}

impl LlamaModels {
    pub async fn new(model_dir: &Path) -> Result<Self> {
        let tokenizer_path = model_dir.join(TOKENIZER_FILENAME);
        let config_path = model_dir.join(CONFIG_FILENAME);

        let dtype = candle::DType::F16;
        let device = auto_device()?;

        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(|e| anyhow!(e.to_string()))?;
        let config: LlamaConfig = serde_json::from_slice(&fs::read(config_path)?)?;

        // TODO: flash attention is supposed to minimize memory read and writes - Do we want to turn it on
        let config = config.into_config(false);
        let cache = llama::Cache::new(true, dtype, &config, &device)?;

        let safetensor_files = load_safetensors(model_dir, MODEL_SAFETENSORS_INDEX_FILE)?;

        let mut tensor_map: HashMap<String, Tensor> = HashMap::new();

        for file in safetensor_files {
            let data = fs::read(file)?;
            let tensors = load_buffer(&data, &device)?;
            for (k, v) in tensors {
                tensor_map.insert(k, v);
            }
        }
        let vb = VarBuilder::from_tensors(tensor_map, dtype, &device);
        let model = Llama::load(vb, &config)?;

        Ok(Self {
            model: Arc::new(model),
            config,
            cache,
            tokenizer,
            device,
        })
    }
}

#[async_trait]
impl InferencingModel for LlamaModels {
    async fn infer(
        &self,
        prompt: String,
        params: wasi_llm::InferencingParams,
    ) -> anyhow::Result<wasi_llm::InferencingResult> {
        let model = Arc::clone(&self.model);
        let config = &self.config;
        let tokenizer = self.tokenizer.clone();
        let mut cache = self.cache.clone();
        // Try to retrieve the End of Sentence (EOS) token ID from config or
        // default to a single EOS token. EOS token is used to determine when to stop.
        let eos_token_id = config.clone().eos_token_id.or_else(|| {
            tokenizer
                .token_to_id(EOS_TOKEN)
                .map(llama::LlamaEosToks::Single)
        });

        let mut tokens = tokenizer
            .encode(prompt, true)
            .map_err(|e| anyhow!(e.to_string()))?
            .get_ids()
            .to_vec();
        let mut rng = rand::rngs::StdRng::from_os_rng();

        let mut logits_processor = {
            let temperature = params.temperature;
            let sampling = if temperature <= 0. {
                Sampling::ArgMax
            } else {
                Sampling::TopKThenTopP {
                    k: params.top_k as usize,
                    p: params.top_p as f64,
                    temperature: params.temperature as f64,
                }
            };
            LogitsProcessor::from_sampling(rng.next_u64(), sampling)
        };

        let mut index_pos = 0;
        let mut tokens_generated = 0;

        for index in 0..params.max_tokens {
            let (context_size, context_index) = if self.cache.use_kv_cache && index > 0 {
                (1, index_pos)
            } else {
                (tokens.len(), 0)
            };
            let ctxt = &tokens[tokens.len().saturating_sub(context_size)..];
            let input = Tensor::new(ctxt, &self.device)?.unsqueeze(0)?;
            let logits = model.forward(&input, context_index, &mut cache)?;
            let logits = logits.squeeze(0)?;
            let logits = if params.repeat_penalty == 1. {
                logits
            } else {
                let start_at = tokens
                    .len()
                    .saturating_sub(params.repeat_penalty_last_n_token_count as usize);
                candle_transformers::utils::apply_repeat_penalty(
                    &logits,
                    params.repeat_penalty,
                    &tokens[start_at..],
                )?
            };
            index_pos += ctxt.len();

            let next_token = logits_processor.sample(&logits)?;
            tokens_generated += 1;
            tokens.push(next_token);

            // Validate if we have reached the end of the token(s)
            match eos_token_id {
                Some(llama::LlamaEosToks::Single(eos_tok_id)) if next_token == eos_tok_id => {
                    break;
                }
                Some(llama::LlamaEosToks::Multiple(ref eos_ids))
                    if eos_ids.contains(&next_token) =>
                {
                    break;
                }
                _ => (),
            }
        }

        let output_text = tokenizer
            .decode(&tokens, true)
            .map_err(|e| anyhow!(e.to_string()))?;

        Ok(wasi_llm::InferencingResult {
            text: output_text,
            usage: InferencingUsage {
                prompt_token_count: tokens.len() as u32,
                generated_token_count: tokens_generated,
            },
        })
    }
}

///  Loads a list of SafeTensors file paths from a given model directory and
///  path to the model index JSON file relative to the model folder.
fn load_safetensors(model_dir: &Path, json_file: &str) -> Result<Vec<std::path::PathBuf>> {
    let json_file = model_dir.join(json_file);
    let json_file = std::fs::File::open(&json_file)
        .with_context(|| format!("Could not read model index file: {json_file:?}"))?;
    let json: serde_json::Value =
        serde_json::from_reader(&json_file).map_err(candle::Error::wrap)?;
    let weight_map = match json.get("weight_map") {
        None => bail!("no weight map in {json_file:?}"),
        Some(serde_json::Value::Object(map)) => map,
        Some(_) => bail!("weight map in {json_file:?} is not a map"),
    };

    let mut safetensors_files = std::collections::HashSet::new();
    for value in weight_map.values() {
        if let Some(file) = value.as_str() {
            safetensors_files.insert(file.to_string());
        }
    }
    let safetensors_files = safetensors_files
        .iter()
        .map(|v| model_dir.join(v))
        .collect::<Vec<_>>();
    Ok(safetensors_files)
}
