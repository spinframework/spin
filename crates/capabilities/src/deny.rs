use crate::{
    AI_MODELS, ALLOWED_OUTBOUND_HOSTS, CAPABILITY_SETS, ENVIRONMENT, FILES, InheritConfiguration,
    KEY_VALUE_STORES, SQLITE_DATABASES, VARIABLES,
};
use wac_graph::types::{ItemKind, SubtypeChecker, are_semver_compatible};
use wac_graph::{CompositionGraph, types::Package};

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
/// Imports are matched on the interface they implement, so a named import such as
/// `primary (implements spin:key-value/key-value@3.0.0)` is treated the same as a
/// plain import of that interface; the label itself is irrelevant.
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

    // Selective plug: wire up only imports NOT in the allow list.
    let socket_instantiation = graph.instantiate(dependency_id);

    let types = graph.types();
    let adapter_exports = &types[graph[deny_adapter_id].ty()].exports;

    let mut plug_exports: Vec<(String, String)> = Vec::new();
    let mut cache = Default::default();
    let mut checker = SubtypeChecker::new(&mut cache);
    for (import_name, socket_ty) in &types[graph[dependency_id].ty()].imports {
        let ItemKind::Instance(iface) = socket_ty else {
            continue;
        };

        // Named imports resolve to the interface they implement; plain imports to their own name.
        let iface_name = types[*iface].id.as_deref().unwrap_or(import_name);

        // Skip interfaces that should be allowed (inherited from host).
        if allow.contains(&iface_name) {
            continue;
        }

        let matching_export = adapter_exports
            .get(iface_name)
            .map(|ty| (iface_name, ty))
            .or_else(|| {
                adapter_exports
                    .iter()
                    .find(|(export_name, _)| are_semver_compatible(export_name, iface_name))
                    .map(|(export_name, ty)| (export_name.as_str(), ty))
            });

        if let Some((plug_name, plug_ty)) = matching_export
            && checker
                .is_subtype(*plug_ty, types, *socket_ty, types)
                .is_ok()
        {
            plug_exports.push((plug_name.to_owned(), import_name.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use wac_graph::types::Types;

    const KV: &str = "spin:key-value/key-value@3.0.0";
    const ENV: &str = "wasi:cli/environment@0.2.6";

    // Imports use an empty instance type, which any adapter export trivially satisfies.
    fn component(imports: &[(&str, Option<&str>)]) -> Vec<u8> {
        let mut wat = String::from("(component\n");
        for (name, implements) in imports {
            match implements {
                Some(iface) => wat.push_str(&format!(
                    "  (import \"{name}\" (implements \"{iface}\") (instance))\n"
                )),
                None => wat.push_str(&format!("  (import \"{name}\" (instance))\n")),
            }
        }
        wat.push(')');
        wat::parse_str(&wat).expect("valid WAT")
    }

    // The composed component also carries the deny adapter's own imports (wasi:io etc.),
    // so tests check for specific names rather than an empty import set.
    fn remaining_imports(bytes: &[u8]) -> Vec<String> {
        let mut types = Types::default();
        let package = Package::from_bytes("out", None, bytes, &mut types).expect("valid component");
        types[package.ty()].imports.keys().cloned().collect()
    }

    fn assert_denied(bytes: &[u8], names: &[&str]) {
        let remaining = remaining_imports(bytes);
        for name in names {
            assert!(
                !remaining.iter().any(|r| r == name),
                "`{name}` should have been denied"
            );
        }
    }

    fn assert_retained(bytes: &[u8], names: &[&str]) {
        let remaining = remaining_imports(bytes);
        for name in names {
            assert!(
                remaining.iter().any(|r| r == name),
                "`{name}` should have been retained"
            );
        }
    }

    fn some(sets: &[&str]) -> InheritConfiguration {
        InheritConfiguration::Some(sets.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn plain_import_is_denied() {
        let source = component(&[(KV, None)]);
        let out = apply_deny_adapter(&source, InheritConfiguration::None).unwrap();
        assert_denied(&out, &[KV]);
    }

    #[test]
    fn named_import_is_denied() {
        let source = component(&[("primary", Some(KV))]);
        let out = apply_deny_adapter(&source, InheritConfiguration::None).unwrap();
        assert_denied(&out, &["primary"]);
    }

    #[test]
    fn named_import_is_allowed_when_inherited() {
        let source = component(&[("primary", Some(KV))]);
        let out = apply_deny_adapter(&source, some(&["key_value_stores"])).unwrap();
        assert_eq!(out, source);
    }

    #[test]
    fn multiple_labels_of_same_interface_are_all_denied() {
        let source = component(&[("primary", Some(KV)), ("backup", Some(KV))]);
        let out = apply_deny_adapter(&source, InheritConfiguration::None).unwrap();
        assert_denied(&out, &["primary", "backup"]);
    }

    #[test]
    fn mixed_named_imports_are_filtered_by_interface() {
        let source = component(&[("primary", Some(KV)), ("env", Some(ENV))]);
        let out = apply_deny_adapter(&source, some(&["key_value_stores"])).unwrap();
        assert_retained(&out, &["primary"]);
        assert_denied(&out, &["env"]);
    }

    #[test]
    fn plain_and_named_imports_of_same_interface_are_both_denied() {
        let source = component(&[(KV, None), ("secondary", Some(KV))]);
        let out = apply_deny_adapter(&source, InheritConfiguration::None).unwrap();
        assert_denied(&out, &[KV, "secondary"]);
    }

    #[test]
    fn unknown_interface_is_left_untouched() {
        let source = component(&[("thing", Some("example:unknown/iface@1.0.0"))]);
        let out = apply_deny_adapter(&source, InheritConfiguration::None).unwrap();
        assert_eq!(out, source);
    }

    #[test]
    fn inherit_all_is_passthrough() {
        let source = component(&[(KV, None), ("primary", Some(KV)), ("env", Some(ENV))]);
        let out = apply_deny_adapter(&source, InheritConfiguration::All).unwrap();
        assert_eq!(out, source);
    }
}
