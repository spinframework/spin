use std::collections::BTreeSet;

use wac_graph::types::are_semver_compatible;
use wasmparser::{Parser, Payload};

use crate::CAPABILITY_SETS;

/// Infer the Spin capability sets a component requires, by inspecting its
/// top-level (component) imports.
///
/// Each import is matched against the known capability sets (`ai_models`,
/// `allowed_outbound_hosts`, etc.); every set that matches contributes its name
/// to the result. The returned set is deduplicated and sorted. An empty set
/// means the component imports nothing that maps to a Spin capability.
pub fn required_capabilities(source: &[u8]) -> anyhow::Result<BTreeSet<String>> {
    let mut capabilities = BTreeSet::new();
    let mut depth: u32 = 0;

    for payload in Parser::new(0).parse_all(source) {
        match payload? {
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => {
                depth += 1;
            }
            Payload::End(_) if depth > 0 => {
                depth -= 1;
            }
            Payload::ComponentImportSection(reader) if depth == 0 => {
                for import in reader {
                    let name = import?.name.0;
                    for &(capability, set) in CAPABILITY_SETS {
                        if set.iter().any(|s| are_semver_compatible(name, s)) {
                            capabilities.insert(capability.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(capabilities)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal Wasm component that imports the given interface names.
    fn build_component(import_names: &[&str]) -> Vec<u8> {
        use wasm_encoder::{
            Component, ComponentImportSection, ComponentTypeRef, ComponentTypeSection, InstanceType,
        };

        let mut component = Component::new();

        // Define one empty instance type to reference from all imports.
        let mut types = ComponentTypeSection::new();
        types.instance(&InstanceType::new());
        component.section(&types);

        let mut imports = ComponentImportSection::new();
        for name in import_names {
            imports.import(name, ComponentTypeRef::Instance(0));
        }
        component.section(&imports);

        component.finish()
    }

    fn caps(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_matching_imports_returns_empty() {
        let bytes = build_component(&["some:unknown/interface@1.0.0"]);
        assert!(required_capabilities(&bytes).unwrap().is_empty());
    }

    #[test]
    fn empty_component_returns_empty() {
        let bytes = wasm_encoder::Component::new().finish();
        assert!(required_capabilities(&bytes).unwrap().is_empty());
    }

    #[test]
    fn single_ai_models_import() {
        let bytes = build_component(&["fermyon:spin/llm@2.0.0"]);
        assert_eq!(required_capabilities(&bytes).unwrap(), caps(&["ai_models"]));
    }

    #[test]
    fn single_allowed_outbound_hosts_import() {
        let bytes = build_component(&["wasi:http/outgoing-handler@0.2.6"]);
        assert_eq!(
            required_capabilities(&bytes).unwrap(),
            caps(&["allowed_outbound_hosts"])
        );
    }

    #[test]
    fn multiple_capabilities_deduped() {
        let bytes = build_component(&[
            "fermyon:spin/llm@2.0.0",
            "wasi:http/outgoing-handler@0.2.6",
            "wasi:sockets/tcp@0.2.6",
            "fermyon:spin/variables@2.0.0",
        ]);
        assert_eq!(
            required_capabilities(&bytes).unwrap(),
            caps(&["ai_models", "allowed_outbound_hosts", "variables"])
        );
    }

    #[test]
    fn all_capability_sets_detected() {
        let bytes = build_component(&[
            "fermyon:spin/llm@2.0.0",           // ai_models
            "wasi:http/outgoing-handler@0.2.6", // allowed_outbound_hosts
            "wasi:cli/environment@0.2.6",       // environment
            "wasi:filesystem/preopens@0.2.6",   // files
            "fermyon:spin/key-value@2.0.0",     // key_value_stores
            "fermyon:spin/sqlite@2.0.0",        // sqlite_databases
            "fermyon:spin/variables@2.0.0",     // variables
        ]);
        assert_eq!(
            required_capabilities(&bytes).unwrap(),
            caps(&[
                "ai_models",
                "allowed_outbound_hosts",
                "environment",
                "files",
                "key_value_stores",
                "sqlite_databases",
                "variables",
            ])
        );
    }

    #[test]
    fn duplicate_set_entries_are_deduped() {
        let bytes =
            build_component(&["wasi:http/outgoing-handler@0.2.6", "wasi:sockets/tcp@0.2.6"]);
        // Both map to allowed_outbound_hosts — should appear once.
        assert_eq!(
            required_capabilities(&bytes).unwrap(),
            caps(&["allowed_outbound_hosts"])
        );
    }

    #[test]
    fn mixed_known_and_unknown_imports() {
        let bytes = build_component(&[
            "fermyon:spin/llm@2.0.0",
            "some:unknown/thing@1.0.0",
            "wasi:cli/environment@0.2.6",
        ]);
        assert_eq!(
            required_capabilities(&bytes).unwrap(),
            caps(&["ai_models", "environment"])
        );
    }
}
