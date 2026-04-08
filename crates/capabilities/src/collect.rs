use std::collections::BTreeSet;

use wac_graph::types::are_semver_compatible;
use wasmparser::{Parser, Payload};

use crate::{InheritConfiguration, CAPABILITY_SETS};

impl InheritConfiguration {
    /// Collect iterates the imports of a Wasm component source and determines where each import
    /// is part of one of the capabilities sets (i.e. AI_MODELS, ALLOWED_OUTBOUND_HOSTS, etc.).
    /// For whichever set it is matched to, the lowercase name is appended to `capabilities` which
    /// is returned as `InheritConfiguration::Some(...)`. If nothing matches `InheritConfiguration::None`
    /// is returned.
    pub fn collect(source: &[u8]) -> anyhow::Result<Option<Self>> {
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
                                capabilities.insert(capability);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if capabilities.is_empty() {
            Ok(None)
        } else {
            Ok(Some(InheritConfiguration::Some(
                capabilities.into_iter().map(String::from).collect(),
            )))
        }
    }
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

    #[test]
    fn no_matching_imports_returns_none() {
        let bytes = build_component(&["some:unknown/interface@1.0.0"]);
        let result = InheritConfiguration::collect(&bytes).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn empty_component_returns_none() {
        let component = wasm_encoder::Component::new();
        let bytes = component.finish();
        let result = InheritConfiguration::collect(&bytes).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn single_ai_models_import() {
        let bytes = build_component(&["fermyon:spin/llm@2.0.0"]);
        let result = InheritConfiguration::collect(&bytes).unwrap();
        let InheritConfiguration::Some(caps) = result.unwrap() else {
            panic!("expected Some capabilities");
        };
        assert_eq!(caps, vec!["ai_models"]);
    }

    #[test]
    fn single_allowed_outbound_hosts_import() {
        let bytes = build_component(&["wasi:http/outgoing-handler@0.2.6"]);
        let result = InheritConfiguration::collect(&bytes).unwrap();
        let InheritConfiguration::Some(caps) = result.unwrap() else {
            panic!("expected Some capabilities");
        };
        assert_eq!(caps, vec!["allowed_outbound_hosts"]);
    }

    #[test]
    fn multiple_capabilities_deduped_and_sorted() {
        let bytes = build_component(&[
            "fermyon:spin/llm@2.0.0",
            "wasi:http/outgoing-handler@0.2.6",
            "wasi:sockets/tcp@0.2.6",
            "fermyon:spin/variables@2.0.0",
        ]);
        let result = InheritConfiguration::collect(&bytes).unwrap();
        let InheritConfiguration::Some(caps) = result.unwrap() else {
            panic!("expected Some capabilities");
        };
        assert_eq!(
            caps,
            vec!["ai_models", "allowed_outbound_hosts", "variables"]
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
        let result = InheritConfiguration::collect(&bytes).unwrap();
        let InheritConfiguration::Some(caps) = result.unwrap() else {
            panic!("expected Some capabilities");
        };
        assert_eq!(
            caps,
            vec![
                "ai_models",
                "allowed_outbound_hosts",
                "environment",
                "files",
                "key_value_stores",
                "sqlite_databases",
                "variables",
            ]
        );
    }

    #[test]
    fn duplicate_set_entries_are_deduped() {
        let bytes =
            build_component(&["wasi:http/outgoing-handler@0.2.6", "wasi:sockets/tcp@0.2.6"]);
        let result = InheritConfiguration::collect(&bytes).unwrap();
        let InheritConfiguration::Some(caps) = result.unwrap() else {
            panic!("expected Some capabilities");
        };
        // Both map to allowed_outbound_hosts — should appear once.
        assert_eq!(caps, vec!["allowed_outbound_hosts"]);
    }

    #[test]
    fn mixed_known_and_unknown_imports() {
        let bytes = build_component(&[
            "fermyon:spin/llm@2.0.0",
            "some:unknown/thing@1.0.0",
            "wasi:cli/environment@0.2.6",
        ]);
        let result = InheritConfiguration::collect(&bytes).unwrap();
        let InheritConfiguration::Some(caps) = result.unwrap() else {
            panic!("expected Some capabilities");
        };
        assert_eq!(caps, vec!["ai_models", "environment"]);
    }
}
