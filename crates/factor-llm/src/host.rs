use spin_world::v1::llm::{self as v1};
use spin_world::v2::llm::{self as v2};
use tracing::field::Empty;
use tracing::{instrument, Level};

use crate::InstanceState;

impl v2::Host for InstanceState {
    #[instrument(name = "spin_llm.infer", skip(self, prompt), err(level = Level::INFO), fields(otel.kind = "client", llm.backend = Empty))]
    async fn infer(
        &mut self,
        model: v2::InferencingModel,
        prompt: String,
        params: Option<v2::InferencingParams>,
    ) -> Result<v2::InferencingResult, v2::Error> {
        self.otel_context.reparent_tracing_span();

        if !self.allowed_models.contains(&model) {
            return Err(access_denied_error(&model));
        }
        let mut engine = self.engine.lock().await;
        tracing::Span::current().record("llm.backend", engine.summary());
        engine
            .infer(
                model,
                prompt,
                params.unwrap_or(v2::InferencingParams {
                    max_tokens: 100,
                    repeat_penalty: 1.1,
                    repeat_penalty_last_n_token_count: 64,
                    temperature: 0.8,
                    top_k: 40,
                    top_p: 0.9,
                }),
            )
            .await
    }

    #[instrument(name = "spin_llm.generate_embeddings", skip(self, data), err(level = Level::INFO), fields(otel.kind = "client", llm.backend = Empty))]
    async fn generate_embeddings(
        &mut self,
        model: v1::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<v2::EmbeddingsResult, v2::Error> {
        self.otel_context.reparent_tracing_span();

        if !self.allowed_models.contains(&model) {
            return Err(access_denied_error(&model));
        }
        let mut engine = self.engine.lock().await;
        tracing::Span::current().record("llm.backend", engine.summary());
        engine.generate_embeddings(model, data).await
    }

    fn convert_error(&mut self, error: v2::Error) -> anyhow::Result<v2::Error> {
        Ok(error)
    }
}

impl v1::Host for InstanceState {
    async fn infer(
        &mut self,
        model: v1::InferencingModel,
        prompt: String,
        params: Option<v1::InferencingParams>,
    ) -> Result<v1::InferencingResult, v1::Error> {
        <Self as v2::Host>::infer(self, model, prompt, params.map(Into::into))
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn generate_embeddings(
        &mut self,
        model: v1::EmbeddingModel,
        data: Vec<String>,
    ) -> Result<v1::EmbeddingsResult, v1::Error> {
        <Self as v2::Host>::generate_embeddings(self, model, data)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    fn convert_error(&mut self, error: v1::Error) -> anyhow::Result<v1::Error> {
        Ok(error)
    }
}

fn access_denied_error(model: &str) -> v2::Error {
    v2::Error::InvalidInput(format!(
        "The component does not have access to use '{model}'. To give the component access, add '{model}' to the 'ai_models' key for the component in your spin.toml manifest"
    ))
}
