use crate::{
    AI_MODELS, ALLOWED_OUTBOUND_HOSTS, CAPABILITY_SETS, ENVIRONMENT, FILES, InheritConfiguration, KEY_VALUE_STORES, SQLITE_DATABASES, VARIABLES
};
use wac_graph::types::{are_semver_compatible, SubtypeChecker};
use wac_graph::{types::Package, CompositionGraph};

/// Composes a deny adapter into a Wasm component to block host capabilities that
/// are not explicitly inherited.
///
/// Given the raw bytes of a Wasm component (`source`) and an [`InheritConfiguration`]
/// describing which capability sets should remain accessible, this function uses
/// `wac-graph` to wire a bundled deny-all adapter into the component's imports.
/// Interfaces listed in the allow set (derived from `inherits`) are left untouched
/// so the host can satisfy them at runtime; all other matching imports are fulfilled
/// by the deny adapter, which traps on any call.
///
/// If the deny adapter has no exports that match the component's imports (i.e. no
/// plugging is needed), the original `source` bytes are returned unchanged.
pub fn apply_deny_adapter(
    source: &[u8],
    inherits: InheritConfiguration,
) -> anyhow::Result<Vec<u8>> {
    let allow = allow_list(inherits);

    const SPIN_DENY_ADAPTER_BYTES: &[u8] = include_bytes!("../deny_adapter.wasm");

    let mut graph = CompositionGraph::new();

    let dependency_package = Package::from_bytes("dependency", None, source, graph.types_mut())?;

    let dependency_id = graph.register_package(dependency_package)?;

    let deny_adapter_package = Package::from_bytes(
        "spin-deny-all-adapter",
        None,
        SPIN_DENY_ADAPTER_BYTES,
        graph.types_mut(),
    )?;

    let deny_adapter_id = graph.register_package(deny_adapter_package)?;

    // Selective plug: wire up only exports NOT in the allow list.
    let socket_instantiation = graph.instantiate(dependency_id);

    let mut plug_exports: Vec<(String, String)> = Vec::new();
    let mut cache = Default::default();
    let mut checker = SubtypeChecker::new(&mut cache);
    for (name, plug_ty) in &graph.types()[graph[deny_adapter_id].ty()].exports {
        // Skip interfaces that should be allowed (inherited from host).
        if allow.iter().any(|a| *a == name) {
            continue;
        }

        let matching_import = graph.types()[graph[dependency_id].ty()]
            .imports
            .get(name)
            .map(|ty| (name.clone(), ty))
            .or_else(|| {
                graph.types()[graph[dependency_id].ty()]
                    .imports
                    .iter()
                    .find(|(import_name, _)| are_semver_compatible(name, import_name))
                    .map(|(import_name, ty)| (import_name.clone(), ty))
            });

        if let Some((socket_name, socket_ty)) = matching_import {
            if checker
                .is_subtype(*plug_ty, graph.types(), *socket_ty, graph.types())
                .is_ok()
            {
                plug_exports.push((name.clone(), socket_name));
            }
        }
    }

    if plug_exports.is_empty() {
        // No plugging needed — return the original source as-is.
        return Ok(source.to_vec());
    }

    let plug_instantiation = graph.instantiate(deny_adapter_id);
    for (plug_name, socket_name) in plug_exports {
        let export = graph.alias_instance_export(plug_instantiation, &plug_name)?;
        graph.set_instantiation_argument(socket_instantiation, &socket_name, export)?;
    }

    // Export all exports from the socket (dependency) component.
    for name in graph.types()[graph[dependency_id].ty()]
        .exports
        .keys()
        .cloned()
        .collect::<Vec<_>>()
    {
        let export = graph.alias_instance_export(socket_instantiation, &name)?;
        graph.export(export, &name)?;
    }

    let bytes = graph.encode(Default::default())?;
    Ok(bytes)
}

fn allow_list(inherits: InheritConfiguration) -> Vec<&'static str> {
    let mut allow = vec![];

    match inherits {
        InheritConfiguration::All => {
            for (_, capability_set) in CAPABILITY_SETS {
                allow.extend_from_slice(capability_set);
            }
        }
        InheritConfiguration::Some(inherits) => {
            for config in inherits {
                match config.as_str() {
                    "ai_models" => allow.extend_from_slice(AI_MODELS),
                    "allowed_outbound_hosts" => allow.extend_from_slice(ALLOWED_OUTBOUND_HOSTS),
                    "environment" => allow.extend_from_slice(ENVIRONMENT),
                    "files" => allow.extend_from_slice(FILES),
                    "key_value_stores" => allow.extend_from_slice(KEY_VALUE_STORES),
                    "sqlite_databases" => allow.extend_from_slice(SQLITE_DATABASES),
                    "variables" => allow.extend_from_slice(VARIABLES),
                    _ => {}
                }
            }
        }
        InheritConfiguration::None => {}
    }

    allow
}
