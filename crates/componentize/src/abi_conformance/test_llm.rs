use std::collections::HashMap;

use anyhow::{ensure, Result};
use serde::Serialize;

use super::llm;

/// Report of which key-value functions a module successfully used, if any
#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct LlmReport {
    pub infer: Result<(), String>,
}

#[derive(Default)]
pub struct Llm {
    inferences: HashMap<(String, String), String>,
    embeddings: HashMap<(String, Vec<String>), Vec<Vec<f32>>>,
}

impl llm::Host for Llm {
    async fn infer(
        &mut self,
        model: llm::InferencingModel,
        prompt: String,
        _params: Option<llm::InferencingParams>,
    ) -> anyhow::Result<Result<llm::InferencingResult, llm::Error>> {
        Ok(self
            .inferences
            .remove(&(model, prompt.clone()))
            .map(|r| llm::InferencingResult {
                text: r,
                usage: llm::InferencingUsage {
                    prompt_token_count: 0,
                    generated_token_count: 0,
                },
            })
            .ok_or_else(|| {
                llm::Error::RuntimeError(format!(
                    "expected {:?}, got {:?}",
                    self.inferences.keys(),
                    prompt
                ))
            }))
    }

    async fn generate_embeddings(
        &mut self,
        model: llm::EmbeddingModel,
        text: Vec<String>,
    ) -> anyhow::Result<Result<llm::EmbeddingsResult, llm::Error>> {
        Ok(self
            .embeddings
            .remove(&(model, text.clone()))
            .map(|r| llm::EmbeddingsResult {
                embeddings: r,
                usage: llm::EmbeddingsUsage {
                    prompt_token_count: 0,
                },
            })
            .ok_or_else(|| {
                llm::Error::RuntimeError(format!(
                    "expected {:?}, got {:?}",
                    self.embeddings.keys(),
                    text
                ))
            }))
    }
}

pub(crate) async fn test(
    engine: &wasmtime::Engine,
    test_config: super::TestConfig,
    pre: &wasmtime::component::InstancePre<super::Context>,
) -> Result<LlmReport> {
    Ok(LlmReport {
        infer: {
            let mut store =
                super::create_store_with_context(engine, test_config.clone(), |context| {
                    context
                        .llm
                        .inferences
                        .insert(("model".into(), "Say hello".into()), "hello".into());
                });

            super::run_command(
                &mut store,
                pre,
                &["llm-infer", "model", "Say hello"],
                |store| {
                    ensure!(
                        store.data().llm.inferences.is_empty(),
                        "expected module to call `llm::infer` exactly once"
                    );

                    Ok(())
                },
            )
            .await
        },
    })
}
