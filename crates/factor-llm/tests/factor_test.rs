use std::collections::HashSet;
use std::sync::Arc;

use spin_factor_llm::{LlmEngine, LlmFactor};
use spin_factors::{anyhow, RuntimeFactors};
use spin_factors_test::{toml, TestEnvironment};
use spin_world::v1::llm::{self as v1};
use spin_world::v2::llm::{self as v2, Host};
use tokio::sync::Mutex;

#[derive(RuntimeFactors)]
struct TestFactors {
    llm: LlmFactor,
}

#[tokio::test]
async fn llm_works() -> anyhow::Result<()> {
    let handle = Box::new(|op| match op {
        Operation::Inference {
            model,
            prompt,
            params,
        } => {
            assert_eq!(model, "llama2-chat");
            assert_eq!(prompt, "some prompt");
            assert_eq!(params.max_tokens, 100);
            Ok(v2::InferencingResult {
                text: "response".to_owned(),
                usage: v2::InferencingUsage {
                    prompt_token_count: 1,
                    generated_token_count: 1,
                },
            }
            .into())
        }
        Operation::Embedding { .. } => {
            todo!("add test for embeddings")
        }
    });
    let factors = TestFactors {
        llm: LlmFactor::new(move || {
            Arc::new(Mutex::new(FakeLLm {
                handle: handle.clone(),
            })) as _
        }),
    };
    let env = TestEnvironment::new(factors).extend_manifest(toml! {
        [component.test-component]
        source = "does-not-exist.wasm"
        ai_models = ["llama2-chat"]
    });
    let mut state = env.build_instance_state().await?;

    assert_eq!(
        &*state.llm.allowed_models,
        &["llama2-chat".to_owned()]
            .into_iter()
            .collect::<HashSet<_>>()
    );

    assert!(matches!(
        state
            .llm
            .infer("unknown-model".into(), "some prompt".into(), Default::default())
            .await,
        Err(v2::Error::InvalidInput(msg)) if msg.contains("The component does not have access to use")
    ));

    state
        .llm
        .infer("llama2-chat".into(), "some prompt".into(), None)
        .await?;
    Ok(())
}

struct FakeLLm {
    handle: Box<dyn Fn(Operation) -> Result<OperationResult, v2::Error> + Sync + Send>,
}

#[allow(dead_code)]
enum Operation {
    Inference {
        model: v1::InferencingModel,
        prompt: String,
        params: v2::InferencingParams,
    },
    Embedding {
        model: v2::EmbeddingModel,
        data: Vec<String>,
    },
}

enum OperationResult {
    Inferencing(v2::InferencingResult),
    Embeddings(v2::EmbeddingsResult),
}

impl From<v2::EmbeddingsResult> for OperationResult {
    fn from(e: v2::EmbeddingsResult) -> Self {
        OperationResult::Embeddings(e)
    }
}

impl From<v2::InferencingResult> for OperationResult {
    fn from(i: v2::InferencingResult) -> Self {
        OperationResult::Inferencing(i)
    }
}

#[async_trait::async_trait]
impl LlmEngine for FakeLLm {
    async fn infer(
        &mut self,
        model: v1::InferencingModel,
        prompt: String,
        params: v2::InferencingParams,
        _max_result_bytes: usize,
    ) -> Result<v2::InferencingResult, v2::Error> {
        let OperationResult::Inferencing(i) = (self.handle)(Operation::Inference {
            model,
            prompt,
            params,
        })?
        else {
            panic!("test incorrectly configured. inferencing operation returned embeddings result")
        };
        Ok(i)
    }

    async fn generate_embeddings(
        &mut self,
        model: v2::EmbeddingModel,
        data: Vec<String>,
        _max_result_bytes: usize,
    ) -> Result<v2::EmbeddingsResult, v2::Error> {
        let OperationResult::Embeddings(e) = (self.handle)(Operation::Embedding { model, data })?
        else {
            panic!("test incorrectly configured. embeddings operation returned inferencing result")
        };
        Ok(e)
    }
}
