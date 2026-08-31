//! Composition of HTTP middleware pipelines.
//!
//! This module is deliberately free of any manifest, lockfile, trigger or
//! executor types: it operates purely on component bytes plus the capability
//! configuration each middleware is allowed to inherit, so that any embedding
//! can reuse it regardless of how it discovers and loads middleware.

use anyhow::{Context, bail};
use wac_graph::{CompositionGraph, PackageId, types::Package};

use crate::InheritConfiguration;

/// The interface an HTTP middleware component both imports (to forward a
/// request to the next component in the pipeline) and exports (to receive a
/// request from the previous one).
pub const MIDDLEWARE_HANDLER_INTERFACE: &str = "wasi:http/handler@0.3.0";

/// A middleware component to be composed into a pipeline.
pub struct Middleware {
    /// The middleware component (or module, which will be componentized).
    pub source: Vec<u8>,
    /// The host capabilities this middleware may inherit from the application.
    pub inherit: InheritConfiguration,
}

/// Composes `middlewares` into a pipeline in front of the `primary` component.
///
/// `middlewares` is ordered from **outermost** (first to receive a request) to
/// **innermost** (last before the primary component). Each middleware has the
/// deny adapter applied according to its [`InheritConfiguration`] before being
/// wired into the pipeline.
///
/// The composed component exports [`MIDDLEWARE_HANDLER_INTERFACE`], backed by
/// the outermost middleware. If `middlewares` is empty, `primary` is returned
/// unchanged.
pub fn compose_middleware_pipeline(
    primary: Vec<u8>,
    middlewares: Vec<Middleware>,
) -> anyhow::Result<Vec<u8>> {
    if middlewares.is_empty() {
        return Ok(primary);
    }

    let mut graph = CompositionGraph::new();
    let mut package_ids: Vec<PackageId> = Vec::new();

    // Register middleware packages (outermost → innermost order).
    for (index, middleware) in middlewares.into_iter().enumerate() {
        let bytes = spin_componentize::componentize_if_necessary(&middleware.source)
            .context("failed to componentize")?;
        let bytes = spin_capabilities::apply_deny_adapter(&bytes, middleware.inherit)?;
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
        MIDDLEWARE_HANDLER_INTERFACE,
        MIDDLEWARE_HANDLER_INTERFACE,
    )?;

    // Export the outermost handler as the composed component's export.
    graph.export(outermost_export, MIDDLEWARE_HANDLER_INTERFACE)?;

    Ok(graph.encode(Default::default())?)
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
