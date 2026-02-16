//! Wrapper component generation.
//!
//! Generates a WebAssembly component that imports a filtered `get-environment`
//! function from the isolator and the original `wasi:cli/environment` instance
//! (for `get-arguments` and `initial-cwd`), then re-exports them bundled as a
//! `wasi:cli/environment` instance.
//!
//! Each target component gets its own wrapper that bridges:
//! - Imports: `wasi:cli/environment@{version}` (passthrough), `environment-{name}-get-environment`
//! - Export: `wasi:cli/environment@{version}` instance
//!
//! Note that these wrappers don't incur any runtime overhead: they get fully optimized
//! away by the component linker.

use anyhow::Result;
use wasm_encoder::{
    Alias, Component, ComponentAliasSection, ComponentExportKind, ComponentExportSection,
    ComponentImportSection, ComponentInstanceSection, ComponentOuterAliasKind, ComponentTypeRef,
    ComponentTypeSection, ComponentValType, InstanceType, PrimitiveValType,
};

/// Build a wrapper component that imports a filtered `get-environment` from
/// the isolator and `get-arguments`/`initial-cwd` from the original
/// `wasi:cli/environment` instance, then exports them bundled as a
/// `wasi:cli/environment` instance.
pub fn build_env_wrapper_component(target_name: &str, wasi_env_version: &str) -> Result<Vec<u8>> {
    let mut component = Component::new();
    let mut types = ComponentTypeSection::new();

    let tuple_idx = types.len();
    types.defined_type().tuple([
        ComponentValType::Primitive(PrimitiveValType::String),
        ComponentValType::Primitive(PrimitiveValType::String),
    ]);

    let list_tss_idx = types.len();
    types.defined_type().list(ComponentValType::Type(tuple_idx));

    let list_s_idx = types.len();
    types
        .defined_type()
        .list(ComponentValType::Primitive(PrimitiveValType::String));

    let option_s_idx = types.len();
    types
        .defined_type()
        .option(ComponentValType::Primitive(PrimitiveValType::String));

    let get_env_idx = types.len();
    types
        .function()
        .params::<[(&str, ComponentValType); 0], ComponentValType>([])
        .result(Some(ComponentValType::Type(list_tss_idx)));

    let get_args_idx = types.len();
    types
        .function()
        .params::<[(&str, ComponentValType); 0], ComponentValType>([])
        .result(Some(ComponentValType::Type(list_s_idx)));

    let get_cwd_idx = types.len();
    types
        .function()
        .params::<[(&str, ComponentValType); 0], ComponentValType>([])
        .result(Some(ComponentValType::Type(option_s_idx)));

    let mut env_instance = InstanceType::new();
    env_instance.alias(Alias::Outer {
        kind: ComponentOuterAliasKind::Type,
        count: 1,
        index: get_env_idx,
    });
    env_instance.alias(Alias::Outer {
        kind: ComponentOuterAliasKind::Type,
        count: 1,
        index: get_args_idx,
    });
    env_instance.alias(Alias::Outer {
        kind: ComponentOuterAliasKind::Type,
        count: 1,
        index: get_cwd_idx,
    });
    env_instance.export("get-environment", ComponentTypeRef::Func(0));
    env_instance.export("get-arguments", ComponentTypeRef::Func(1));
    env_instance.export("initial-cwd", ComponentTypeRef::Func(2));
    let env_instance_idx = types.len();
    types.instance(&env_instance);

    component.section(&types);

    let mut imports = ComponentImportSection::new();
    imports.import(
        &format!("wasi:cli/environment@{wasi_env_version}"),
        ComponentTypeRef::Instance(env_instance_idx),
    );
    imports.import(
        &format!("environment-{target_name}-get-environment"),
        ComponentTypeRef::Func(get_env_idx),
    );
    component.section(&imports);

    // Alias get-arguments and initial-cwd from the imported instance
    let mut aliases = ComponentAliasSection::new();
    aliases.alias(Alias::InstanceExport {
        instance: 0,
        kind: ComponentExportKind::Func,
        name: "get-arguments",
    });
    aliases.alias(Alias::InstanceExport {
        instance: 0,
        kind: ComponentExportKind::Func,
        name: "initial-cwd",
    });
    component.section(&aliases);

    // Bundle: func 0 = get-environment (imported), func 1 = get-arguments (aliased),
    // func 2 = initial-cwd (aliased)
    let mut instances = ComponentInstanceSection::new();
    instances.export_items([
        ("get-environment", ComponentExportKind::Func, 0),
        ("get-arguments", ComponentExportKind::Func, 1),
        ("initial-cwd", ComponentExportKind::Func, 2),
    ]);
    component.section(&instances);

    let mut exports = ComponentExportSection::new();
    exports.export(
        &format!("wasi:cli/environment@{wasi_env_version}"),
        ComponentExportKind::Instance,
        1,
        Some(ComponentTypeRef::Instance(env_instance_idx)),
    );
    component.section(&exports);

    let bytes = component.finish();

    #[cfg(debug_assertions)]
    wasmparser::validate(&bytes)
        .map_err(|e| anyhow::anyhow!("generated env wrapper component is not valid: {e}"))?;

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_wrapper() {
        let bytes =
            build_env_wrapper_component("main", "0.2.3").expect("wrapper generation failed");
        wasmparser::validate(&bytes).expect("wrapper component should be valid");
    }
}
