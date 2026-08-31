use anyhow::{Context, bail};

use std::collections::HashMap;

use spin_compose::{DependencyLike, middleware::Middleware};
use spin_factors_executor::{
    TriggerDependenciesComposer, TriggerDependency, TriggerDependencyData,
};

#[derive(Default)]
pub(crate) struct HttpMiddlewareComposer;

#[spin_core::async_trait]
impl TriggerDependenciesComposer for HttpMiddlewareComposer {
    async fn compose_trigger_dependencies(
        &self,
        trigger_dependencies: &HashMap<String, Vec<TriggerDependency>>,
        component: Vec<u8>,
    ) -> anyhow::Result<Vec<u8>> {
        let Some(middlewares) = trigger_dependencies.get("middleware") else {
            return Ok(component);
        };
        if trigger_dependencies.len() > 1 {
            bail!("the HTTP trigger's only allowed trigger dependency is `middleware`");
        }
        if middlewares.is_empty() {
            return Ok(component);
        }

        let pipeline = load_pipeline(middlewares).await?;
        spin_compose::middleware::compose_middleware_pipeline(component, pipeline)
    }
}

/// Read the middleware sources referenced by the trigger's dependencies, in
/// outermost → innermost order.
async fn load_pipeline(middlewares: &[TriggerDependency]) -> anyhow::Result<Vec<Middleware>> {
    let mut pipeline = Vec::with_capacity(middlewares.len());

    for dep in middlewares {
        let source = match &dep.data {
            TriggerDependencyData::InMemory(data) => data.clone(),
            TriggerDependencyData::OnDisk(path) => tokio::fs::read(path)
                .await
                .with_context(|| format!("reading middleware from {}", path.display()))?,
        };
        pipeline.push(Middleware {
            source,
            inherit: dep.dependency.inherit(),
        });
    }

    Ok(pipeline)
}
