//! Isolator component generation.
//!
//! Constructs a WebAssembly component that:
//! - Imports `wasi:cli/environment@{version}` from the host
//! - Embeds a core filter module (from `core_module`)
//! - Exports per-component isolated environment functions

use anyhow::{Context, Result};
use wasm_encoder::{
    Alias, CanonicalOption, ComponentBuilder, ComponentExportKind, ComponentOuterAliasKind,
    ComponentTypeRef, ComponentValType, ExportKind, InstanceType, ModuleArg, PrimitiveValType,
};

use crate::core_module::{build_memory_module, generate_env_filter_module};

/// Information about a component that needs environment isolation.
pub struct IsolationTarget {
    /// Logical name for this component (e.g., "main", "dep").
    pub name: String,
    /// Environment variable prefix to filter by.
    pub prefix: String,
}

/// Generate an isolator component that filters environment variables per-component.
///
/// The generated component:
/// 1. Imports `wasi:cli/environment@{wasi_env_version}`
/// 2. Contains a core module that filters env vars by prefix
/// 3. Exports `environment-{name}-get-environment` etc. for each target
pub fn generate_isolator(targets: &[IsolationTarget], wasi_env_version: &str) -> Result<Vec<u8>> {
    anyhow::ensure!(
        !targets.is_empty(),
        "at least one isolation target required"
    );

    let prefixes: Vec<&str> = targets.iter().map(|t| t.prefix.as_str()).collect();
    let core_module_bytes = generate_env_filter_module(&prefixes);

    #[cfg(debug_assertions)]
    wasmparser::validate(&core_module_bytes)
        .context("generated filter core module is not valid Wasm")?;

    let mut builder = ComponentBuilder::default();

    // --- Component-level types ---

    // Type 0: tuple<string, string>
    let (tuple_ss_type, enc) = builder.type_defined(None);
    enc.tuple([
        ComponentValType::Primitive(PrimitiveValType::String),
        ComponentValType::Primitive(PrimitiveValType::String),
    ]);

    // Type 1: list<tuple<string, string>>
    let (list_tss_type, enc) = builder.type_defined(None);
    enc.list(ComponentValType::Type(tuple_ss_type));

    // Type 2: list<string>
    let (list_s_type, enc) = builder.type_defined(None);
    enc.list(ComponentValType::Primitive(PrimitiveValType::String));

    // Type 3: option<string>
    let (option_s_type, enc) = builder.type_defined(None);
    enc.option(ComponentValType::Primitive(PrimitiveValType::String));

    // Type 4: func get-environment() -> list<tuple<string, string>>
    let (get_env_func_type, mut enc) = builder.type_function(None);
    enc.params::<[(&str, ComponentValType); 0], ComponentValType>([])
        .result(Some(ComponentValType::Type(list_tss_type)));

    // Type 5: func get-arguments() -> list<string>
    let (get_args_func_type, mut enc) = builder.type_function(None);
    enc.params::<[(&str, ComponentValType); 0], ComponentValType>([])
        .result(Some(ComponentValType::Type(list_s_type)));

    // Type 6: func initial-cwd() -> option<string>
    let (get_cwd_func_type, mut enc) = builder.type_function(None);
    enc.params::<[(&str, ComponentValType); 0], ComponentValType>([])
        .result(Some(ComponentValType::Type(option_s_type)));

    // Type 7: instance type for wasi:cli/environment
    let mut env_instance_type = InstanceType::new();
    env_instance_type.alias(Alias::Outer {
        kind: ComponentOuterAliasKind::Type,
        count: 1,
        index: get_env_func_type,
    });
    env_instance_type.alias(Alias::Outer {
        kind: ComponentOuterAliasKind::Type,
        count: 1,
        index: get_args_func_type,
    });
    env_instance_type.alias(Alias::Outer {
        kind: ComponentOuterAliasKind::Type,
        count: 1,
        index: get_cwd_func_type,
    });
    env_instance_type.export("get-environment", ComponentTypeRef::Func(0));
    env_instance_type.export("get-arguments", ComponentTypeRef::Func(1));
    env_instance_type.export("initial-cwd", ComponentTypeRef::Func(2));
    let env_instance_type_idx = builder.type_instance(None, &env_instance_type);

    // --- Import wasi:cli/environment ---
    let import_name = format!("wasi:cli/environment@{wasi_env_version}");
    let host_env_instance = builder.import(
        &import_name,
        ComponentTypeRef::Instance(env_instance_type_idx),
    );

    // --- Alias functions from imported instance ---
    let host_get_env = builder.alias_export(
        host_env_instance,
        "get-environment",
        ComponentExportKind::Func,
    );
    let host_get_args = builder.alias_export(
        host_env_instance,
        "get-arguments",
        ComponentExportKind::Func,
    );
    let host_get_cwd =
        builder.alias_export(host_env_instance, "initial-cwd", ComponentExportKind::Func);

    // --- Embed core modules ---
    let total_prefix_bytes: usize = targets.iter().map(|t| t.prefix.len()).sum();
    let aux_module = build_memory_module(total_prefix_bytes as u32);
    let aux_module_idx = builder.core_module(Some("aux"), &aux_module);
    let filter_module_idx = builder.core_module_raw(Some("filter"), &core_module_bytes);

    // --- Instantiate aux module (provides memory + realloc) ---
    let aux_instance =
        builder.core_instantiate(Some("aux"), aux_module_idx, Vec::<(&str, ModuleArg)>::new());

    // Alias memory, realloc, and reset from aux
    let memory = builder.core_alias_export(None, aux_instance, "memory", ExportKind::Memory);
    let realloc = builder.core_alias_export(None, aux_instance, "realloc", ExportKind::Func);
    let reset = builder.core_alias_export(None, aux_instance, "reset", ExportKind::Func);

    // --- Lower host functions ---
    let lowered_get_env = builder.lower_func(
        None,
        host_get_env,
        [
            CanonicalOption::Memory(memory),
            CanonicalOption::Realloc(realloc),
        ],
    );
    let lowered_get_args = builder.lower_func(
        None,
        host_get_args,
        [
            CanonicalOption::Memory(memory),
            CanonicalOption::Realloc(realloc),
        ],
    );
    let lowered_get_cwd = builder.lower_func(
        None,
        host_get_cwd,
        [
            CanonicalOption::Memory(memory),
            CanonicalOption::Realloc(realloc),
        ],
    );

    // --- Build import instance for filter module ---
    let host_for_filter = builder.core_instantiate_exports(
        Some("host-for-filter"),
        vec![
            ("memory", ExportKind::Memory, memory),
            ("get-environment", ExportKind::Func, lowered_get_env),
            ("get-arguments", ExportKind::Func, lowered_get_args),
            ("initial-cwd", ExportKind::Func, lowered_get_cwd),
            ("realloc", ExportKind::Func, realloc),
            ("reset", ExportKind::Func, reset),
        ],
    );

    // --- Instantiate filter module ---
    let filter_instance = builder.core_instantiate(
        Some("filter"),
        filter_module_idx,
        vec![("host", ModuleArg::Instance(host_for_filter))],
    );

    // --- For each target, lift the filtered functions and export ---
    for (i, target) in targets.iter().enumerate() {
        let filtered_get_env = builder.core_alias_export(
            None,
            filter_instance,
            &format!("get-environment-{i}"),
            ExportKind::Func,
        );

        let passthrough_get_args =
            builder.core_alias_export(None, filter_instance, "get-arguments", ExportKind::Func);
        let passthrough_get_cwd =
            builder.core_alias_export(None, filter_instance, "initial-cwd", ExportKind::Func);

        // Lift core functions back to component functions
        let lifted_get_env = builder.lift_func(
            None,
            filtered_get_env,
            get_env_func_type,
            [
                CanonicalOption::Memory(memory),
                CanonicalOption::Realloc(realloc),
            ],
        );
        let lifted_get_args = builder.lift_func(
            None,
            passthrough_get_args,
            get_args_func_type,
            [
                CanonicalOption::Memory(memory),
                CanonicalOption::Realloc(realloc),
            ],
        );
        let lifted_get_cwd = builder.lift_func(
            None,
            passthrough_get_cwd,
            get_cwd_func_type,
            [
                CanonicalOption::Memory(memory),
                CanonicalOption::Realloc(realloc),
            ],
        );

        let export_name = format!("environment-{}", target.name);

        builder.export(
            &format!("{export_name}-get-environment"),
            ComponentExportKind::Func,
            lifted_get_env,
            None,
        );
        builder.export(
            &format!("{export_name}-get-arguments"),
            ComponentExportKind::Func,
            lifted_get_args,
            None,
        );
        builder.export(
            &format!("{export_name}-get-cwd"),
            ComponentExportKind::Func,
            lifted_get_cwd,
            None,
        );
    }

    let bytes = builder.finish();

    #[cfg(debug_assertions)]
    wasmparser::validate(&bytes).context("generated isolator component is not valid")?;

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_module_validates() {
        let m = build_memory_module(0);
        let bytes = m.finish();
        wasmparser::validate(&bytes).expect("memory module should be valid");
    }

    #[test]
    fn test_generate_isolator_basic() {
        let targets = vec![
            IsolationTarget {
                name: "main".to_string(),
                prefix: "main_".to_string(),
            },
            IsolationTarget {
                name: "dep".to_string(),
                prefix: "dep_".to_string(),
            },
        ];
        generate_isolator(&targets, "0.2.3").expect("generated isolator component is not valid");
    }
}
