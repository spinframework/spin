use anyhow::{bail, Context};
use wac_graph::{types::Package, CompositionGraph, PackageId};

use std::collections::HashMap;

use spin_factors_executor::{Complication, ComplicationData, Complicator};

#[derive(Default)]
pub(crate) struct HttpMiddlewareComplicator;

#[spin_core::async_trait]
impl Complicator for HttpMiddlewareComplicator {
    async fn complicate(
        &self,
        complications: &HashMap<String, Vec<Complication>>,
        component: Vec<u8>,
    ) -> anyhow::Result<Vec<u8>> {
        let Some(middlewares) = complications.get("middleware") else {
            return Ok(component);
        };
        if complications.len() > 1 {
            bail!("the HTTP trigger's only allowed complication is `middleware`");
        }
        if middlewares.is_empty() {
            return Ok(component);
        }

        let middleware_blobs = middlewares.iter().map(|cm| &cm.data);
        compose_middlewares(component, middleware_blobs).await
    }
}

/// Chain a list of component packages into a middleware pipeline.
///
/// `packages` is ordered from **outermost** (first to receive a request) to
/// **innermost** (the final handler).  Every component except the last must
/// import a name equal to `import_name` and every component must export a name
/// equal to `export_name`.  In the common middleware pattern these are the same
/// (e.g. both `"handle"`), but they can differ if the WIT uses separate names.
///
/// Returns the [`NodeId`] of the alias for the outermost component's export,
/// ready to be passed to [`CompositionGraph::export`].
///
/// # Errors
///
/// Returns an error if fewer than two packages are provided, or if any
/// alias / argument wiring step fails.
fn chain(
    graph: &mut CompositionGraph,
    packages: &[PackageId],
    import_name: &str,
    export_name: &str,
) -> anyhow::Result<wac_graph::NodeId> {
    if packages.len() < 2 {
        bail!("chain requires at least 2 packages, got {}", packages.len());
    }

    // Start from the innermost component (last in the list) and work outward.
    // The innermost component is instantiated first with no wiring — its
    // unsatisfied imports (if any) will become implicit imports of the
    // composed component.
    let mut iter = packages.iter().rev();
    let innermost = *iter.next().unwrap();
    let mut instance = graph.instantiate(innermost);
    let mut upstream_handle = graph.alias_instance_export(instance, export_name)?;

    // For each remaining component (moving outward), instantiate it and
    // wire the previous component's export into its import.
    for &pkg in iter {
        instance = graph.instantiate(pkg);
        graph.set_instantiation_argument(instance, import_name, upstream_handle)?;
        upstream_handle = graph.alias_instance_export(instance, export_name)?;
    }

    Ok(upstream_handle)
}

async fn compose_middlewares<'a>(
    primary: Vec<u8>,
    middleware_blobs: impl Iterator<Item = &'a ComplicationData>,
) -> anyhow::Result<Vec<u8>> {
    const MW_HANDLER_INTERFACE: &str = "wasi:http/handler@0.3.0-rc-2026-03-15";

    let mut graph = CompositionGraph::new();
    let mut package_ids: Vec<PackageId> = Vec::new();

    // Register middleware packages (outermost → innermost order).
    for (index, blob) in middleware_blobs.enumerate() {
        let bytes: Vec<u8> = match blob {
            ComplicationData::InMemory(data) => data.clone(),
            ComplicationData::OnDisk(path) => tokio::fs::read(path)
                .await
                .with_context(|| format!("reading middleware from {}", path.display()))?,
        };
        let name = format!("middleware{index}");
        let package = Package::from_bytes(&name, None, bytes, graph.types_mut())
            .context("parsing middleware component")?;
        package_ids.push(graph.register_package(package)?);
    }

    // Register the primary component (innermost in the chain).
    let package = Package::from_bytes("primary", None, primary, graph.types_mut())
        .context("parsing primary component")?;
    package_ids.push(graph.register_package(package)?);

    // Wire the pipeline: outermost middleware → … → primary.
    let outermost_export = chain(
        &mut graph,
        &package_ids,
        MW_HANDLER_INTERFACE,
        MW_HANDLER_INTERFACE,
    )?;

    // Export the outermost handler as the composed component's export.
    graph.export(outermost_export, MW_HANDLER_INTERFACE)?;

    Ok(graph.encode(Default::default())?)
}
