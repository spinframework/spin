use wasm_metadata::Producers;
use wasmparser::{Encoding, ExternalKind, Parser, Payload};
use wit_component::metadata::Bindgen;

// wit-bindgen has used both of these historically.
const CANONICAL_ABI_REALLOC_EXPORTS: &[&str] = &["cabi_realloc", "canonical_abi_realloc"];

/// Stores various bits of info parsed from a Wasm module that are relevant to
/// componentization.
#[derive(Default)]
pub struct ModuleInfo {
    pub bindgen: Option<Bindgen>,
    pub clang_version: Option<String>,
    pub realloc_export: Option<String>,
    pub has_start_export: bool,
}

impl ModuleInfo {
    /// Parses info from the given binary module bytes.
    pub fn from_module(module: &[u8]) -> anyhow::Result<Self> {
        let mut info = Self::default();
        for payload in Parser::new(0).parse_all(module) {
            match payload? {
                Payload::Version { encoding, .. } => {
                    anyhow::ensure!(
                        encoding == Encoding::Module,
                        "ModuleInfo::from_module is only applicable to Modules; got a {encoding:?}"
                    );
                }
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export?;
                        if export.kind == ExternalKind::Func {
                            if CANONICAL_ABI_REALLOC_EXPORTS.contains(&export.name) {
                                tracing::debug!(
                                    "Found canonical ABI realloc export {:?}",
                                    export.name
                                );
                                info.realloc_export = Some(export.name.to_string());
                            } else if export.name == "_start" {
                                tracing::debug!("Found _start export");
                                info.has_start_export = true;
                            }
                        }
                    }
                }
                Payload::CustomSection(c) => {
                    let section_name = c.name();
                    if section_name == "producers" {
                        let producers = Producers::from_bytes(c.data(), c.data_offset())?;
                        if let Some(clang_version) =
                            producers.get("processed-by").and_then(|f| f.get("clang"))
                        {
                            tracing::debug!(clang_version, "Parsed producers.processed-by.clang");
                            info.clang_version = Some(clang_version.to_string());
                        }
                    } else if section_name.starts_with("component-type") {
                        match decode_bindgen_custom_section(section_name, c.data()) {
                            Ok(bindgen) => {
                                tracing::debug!("Parsed bindgen section {section_name:?}");
                                info.bindgen = Some(bindgen);
                            }
                            Err(err) => tracing::warn!(
                                "Error parsing bindgen section {section_name:?}: {err}"
                            ),
                        }
                    }
                }
                _ => (),
            }
        }
        Ok(info)
    }

    /// Returns true if the given module was heuristically probably compiled
    /// with wit-bindgen.
    pub fn probably_uses_wit_bindgen(&self) -> bool {
        // Presence of bindgen metadata is a strong signal
        self.bindgen.is_some() ||
            // A canonical ABI realloc export is a decent signal
            self.realloc_export.is_some()
    }

    /// Returns the wit-bindgen metadata producers processed-by field, if
    /// present.
    pub fn bindgen_processors(&self) -> Option<wasm_metadata::ProducersField<'_>> {
        self.bindgen
            .as_ref()?
            .producers
            .as_ref()?
            .get("processed-by")
    }
}

/// This is a silly workaround for the limited public interface available in
/// [`wit_component::metadata`].
// TODO: Make Bindgen::decode_custom_section public?
fn decode_bindgen_custom_section(name: &str, data: &[u8]) -> anyhow::Result<Bindgen> {
    let mut module = wasm_encoder::Module::new();
    module.section(&wasm_encoder::CustomSection {
        name: name.into(),
        data: data.into(),
    });
    let (_, bindgen) = wit_component::metadata::decode(module.as_slice())?;
    Ok(bindgen)
}
