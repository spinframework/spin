//! Composition of HTTP middleware pipelines.
//!
//! This module is deliberately free of any manifest, lockfile, trigger or
//! executor types: it operates purely on component bytes plus the capability
//! configuration each middleware is allowed to inherit, so that any embedding
//! can reuse it regardless of how it discovers and loads middleware.

use anyhow::{Context, bail};
use semver::Version;
use wac_graph::{CompositionGraph, NodeId, PackageId, types::Package};

use crate::InheritConfiguration;

/// The interface an HTTP middleware component both imports (to forward a
/// request to the next component in the pipeline) and exports (to receive a
/// request from the previous one).
///
/// Any semver-compatible `0.3.x` version of this interface is accepted, and
/// a component's import and export may use different `0.3.x` versions.
pub const MIDDLEWARE_HANDLER_INTERFACE: &str = "wasi:http/handler";

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
/// Each component's [`MIDDLEWARE_HANDLER_INTERFACE`] import and export are
/// located by semver compatibility with `0.3`, so components built against
/// different `0.3.x` versions of `wasi:http` can be mixed. `wac-graph` merges
/// the semver-compatible `wasi:http/types` imports into a single import at the
/// highest version, so all components share one set of HTTP resources.
///
/// The composed component exports [`MIDDLEWARE_HANDLER_INTERFACE`] at the
/// highest `0.3.x` version exported by any component in the pipeline, backed
/// by the outermost middleware. If `middlewares` is empty, `primary` is
/// returned unchanged.
pub fn compose_middleware_pipeline(
    primary: Vec<u8>,
    middlewares: Vec<Middleware>,
) -> anyhow::Result<Vec<u8>> {
    if middlewares.is_empty() {
        return Ok(primary);
    }

    let mut graph = CompositionGraph::new();
    let mut stages = Vec::with_capacity(middlewares.len());

    // Register middleware packages (outermost → innermost order).
    for (index, middleware) in middlewares.into_iter().enumerate() {
        let description = format!("middleware #{}", index + 1);
        let bytes = spin_componentize::componentize_if_necessary(&middleware.source)
            .with_context(|| format!("failed to componentize {description}"))?;
        let bytes = spin_capabilities::apply_deny_adapter(&bytes, middleware.inherit)?;
        let name = format!("middleware{index}");
        let package = Package::from_bytes(&name, None, bytes, graph.types_mut())
            .with_context(|| format!("parsing {description}"))?;
        let world = &graph.types()[package.ty()];
        let import = find_handler(world.imports.keys(), "import", &description)?;
        let export = find_handler(world.exports.keys(), "export", &description)?;
        stages.push(Stage {
            package: graph.register_package(package)?,
            description,
            import,
            export,
        });
    }

    // Register the primary component (innermost in the chain).
    let package = Package::from_bytes("primary", None, primary, graph.types_mut())
        .context("parsing primary component")?;
    let export = find_handler(
        graph.types()[package.ty()].exports.keys(),
        "export",
        "primary component",
    )?;
    let primary = (graph.register_package(package)?, export);

    let composed_export = stages
        .iter()
        .map(|stage| &stage.export)
        .chain([&primary.1])
        .max_by_key(|handler| &handler.version)
        .expect("pipeline always contains the primary component")
        .name
        .clone();

    // Wire the pipeline: outermost middleware → … → primary.
    let outermost_export = chain(&mut graph, primary, &stages)?;

    // Export the outermost handler as the composed component's export.
    graph.export(outermost_export, composed_export)?;

    Ok(graph.encode(Default::default())?)
}

/// A `wasi:http/handler@0.3.x` import or export name found on a component.
struct Handler {
    name: String,
    version: Version,
}

/// A registered middleware component and its handler import/export names.
struct Stage {
    package: PackageId,
    description: String,
    import: Handler,
    export: Handler,
}

/// Finds the single `wasi:http/handler` name on the `0.3` semver track among
/// `names`, which are a component's imports or exports (`kind`).
fn find_handler<'a>(
    names: impl IntoIterator<Item = &'a String>,
    kind: &str,
    description: &str,
) -> anyhow::Result<Handler> {
    let mut handlers = names.into_iter().filter_map(|name| {
        let version = name
            .strip_prefix(MIDDLEWARE_HANDLER_INTERFACE)?
            .strip_prefix('@')?;
        let version = Version::parse(version).ok()?;
        (version.major == 0 && version.minor == 3 && version.pre.is_empty()).then(|| Handler {
            name: name.clone(),
            version,
        })
    });
    let Some(handler) = handlers.next() else {
        bail!("{description} does not {kind} `{MIDDLEWARE_HANDLER_INTERFACE}@0.3.x`");
    };
    if let Some(other) = handlers.next() {
        bail!(
            "{description} has ambiguous `{MIDDLEWARE_HANDLER_INTERFACE}@0.3.x` {kind}s: `{}` and `{}`",
            handler.name,
            other.name
        );
    }
    Ok(handler)
}

/// Chains `middlewares` (ordered **outermost** to **innermost**) in front of
/// the `primary` package, wiring each component's handler export into the
/// handler import of the component before it.
///
/// Returns the [`NodeId`] of the alias for the outermost component's export,
/// ready to be passed to [`CompositionGraph::export`].
fn chain(
    graph: &mut CompositionGraph,
    (primary, primary_export): (PackageId, Handler),
    middlewares: &[Stage],
) -> anyhow::Result<NodeId> {
    // The innermost component's unsatisfied imports (if any) become implicit
    // imports of the composed component.
    let instance = graph.instantiate(primary);
    let mut upstream_handle = graph.alias_instance_export(instance, &primary_export.name)?;

    for stage in middlewares.iter().rev() {
        let instance = graph.instantiate(stage.package);
        graph
            .set_instantiation_argument(instance, &stage.import.name, upstream_handle)
            .with_context(|| {
                format!(
                    "failed to wire `{}` of {}",
                    stage.import.name, stage.description
                )
            })?;
        upstream_handle = graph.alias_instance_export(instance, &stage.export.name)?;
    }

    Ok(upstream_handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wac_graph::types::Types;

    fn http_wit(version: &str) -> String {
        format!(
            r#"
            package wasi:http@{version};

            interface types {{
                resource fields {{ constructor(); }}
                resource request {{ constructor(headers: fields); }}
                resource response {{ constructor(headers: fields); }}
                variant error-code {{ internal-error(option<string>) }}
            }}

            interface handler {{
                use types.{{request, response, error-code}};
                handle: async func(request: request) -> result<response, error-code>;
            }}
            "#
        )
    }

    /// Builds a dummy component whose world contains `items` (e.g.
    /// `export wasi:http/handler@0.3.2;`).
    fn component(items: &[&str]) -> Vec<u8> {
        let mut resolve = wit_parser::Resolve::default();
        for version in ["0.3.0", "0.3.2"] {
            resolve.push_str("http.wit", &http_wit(version)).unwrap();
        }
        let wit = format!(
            "package test:middleware;\nworld test {{\n{}\n}}",
            items.join("\n")
        );
        let package = resolve.push_str("test.wit", &wit).unwrap();
        let world = resolve.select_world(&[package], Some("test")).unwrap();

        let mut module = wit_component::dummy_module(
            &resolve,
            world,
            wit_parser::ManglingAndAbi::Legacy(wit_parser::LiftLowerAbi::Sync),
        );
        wit_component::embed_component_metadata(
            &mut module,
            &resolve,
            world,
            wit_component::StringEncoding::UTF8,
            false,
        )
        .unwrap();
        wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&module)
            .unwrap()
            .encode()
            .unwrap()
    }

    fn service(version: &str) -> Vec<u8> {
        component(&[&format!("export wasi:http/handler@{version};")])
    }

    fn middleware(import_version: &str, export_version: &str) -> Middleware {
        Middleware {
            source: component(&[
                &format!("import wasi:http/handler@{import_version};"),
                &format!("export wasi:http/handler@{export_version};"),
            ]),
            inherit: InheritConfiguration::All,
        }
    }

    /// Returns the sorted import and export names of a composed component.
    fn imports_and_exports(bytes: Vec<u8>) -> (Vec<String>, Vec<String>) {
        let mut types = Types::default();
        let package = Package::from_bytes("composed", None, bytes, &mut types).unwrap();
        let world = &types[package.ty()];
        let mut imports: Vec<_> = world.imports.keys().cloned().collect();
        let mut exports: Vec<_> = world.exports.keys().cloned().collect();
        imports.sort();
        exports.sort();
        (imports, exports)
    }

    fn assert_composes(
        primary: Vec<u8>,
        middlewares: Vec<Middleware>,
        expected_types_import: &str,
        expected_handler_export: &str,
    ) {
        let composed = compose_middleware_pipeline(primary, middlewares).unwrap();
        let (imports, exports) = imports_and_exports(composed);
        assert_eq!(imports, [expected_types_import]);
        assert_eq!(exports, [expected_handler_export]);
    }

    #[test]
    fn all_components_at_0_3_0() {
        assert_composes(
            service("0.3.0"),
            vec![middleware("0.3.0", "0.3.0"), middleware("0.3.0", "0.3.0")],
            "wasi:http/types@0.3.0",
            "wasi:http/handler@0.3.0",
        );
    }

    #[test]
    fn all_components_at_0_3_2() {
        assert_composes(
            service("0.3.2"),
            vec![middleware("0.3.2", "0.3.2"), middleware("0.3.2", "0.3.2")],
            "wasi:http/types@0.3.2",
            "wasi:http/handler@0.3.2",
        );
    }

    #[test]
    fn newer_primary_with_older_middleware() {
        assert_composes(
            service("0.3.2"),
            vec![middleware("0.3.0", "0.3.0")],
            "wasi:http/types@0.3.2",
            "wasi:http/handler@0.3.2",
        );
    }

    #[test]
    fn older_primary_with_newer_middleware() {
        assert_composes(
            service("0.3.0"),
            vec![middleware("0.3.2", "0.3.2")],
            "wasi:http/types@0.3.2",
            "wasi:http/handler@0.3.2",
        );
    }

    #[test]
    fn middleware_with_mixed_import_and_export_versions() {
        assert_composes(
            service("0.3.2"),
            vec![middleware("0.3.0", "0.3.2")],
            "wasi:http/types@0.3.2",
            "wasi:http/handler@0.3.2",
        );
        assert_composes(
            service("0.3.0"),
            vec![
                middleware("0.3.0", "0.3.0"),
                middleware("0.3.0", "0.3.2"),
                middleware("0.3.2", "0.3.0"),
            ],
            "wasi:http/types@0.3.2",
            "wasi:http/handler@0.3.2",
        );
    }

    #[test]
    fn primary_without_0_3_handler_export_fails() {
        let primary = component(&["export run: func();"]);
        let err = compose_middleware_pipeline(primary, vec![middleware("0.3.0", "0.3.0")])
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "primary component does not export `wasi:http/handler@0.3.x`"
        );
    }

    #[test]
    fn middleware_without_0_3_handler_import_fails() {
        let middleware = Middleware {
            source: service("0.3.0"),
            inherit: InheritConfiguration::All,
        };
        let err = compose_middleware_pipeline(service("0.3.0"), vec![middleware])
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "middleware #1 does not import `wasi:http/handler@0.3.x`"
        );
    }

    #[test]
    fn ambiguous_handler_exports_fail() {
        let primary = component(&[
            "export wasi:http/handler@0.3.0;",
            "export wasi:http/handler@0.3.2;",
        ]);
        let err = compose_middleware_pipeline(primary, vec![middleware("0.3.0", "0.3.0")])
            .unwrap_err()
            .to_string();
        assert_eq!(
            err,
            "primary component has ambiguous `wasi:http/handler@0.3.x` exports: \
             `wasi:http/handler@0.3.0` and `wasi:http/handler@0.3.2`"
        );
    }

    #[test]
    fn no_middleware_returns_primary_unchanged() {
        let primary = service("0.3.2");
        let composed = compose_middleware_pipeline(primary.clone(), vec![]).unwrap();
        assert_eq!(composed, primary);
    }
}
