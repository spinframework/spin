use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use spin_common::ui::quoted_path;
use spin_locked_app::locked::LockedApp;
use spin_manifest::schema::v2::AppManifest;

/// A source from which an App may be loaded.
#[derive(Debug, PartialEq, Eq)]
pub enum AppSource {
    File(PathBuf),
    OciRegistry(String),
    BareWasm(PathBuf),
    Unresolvable(String),
    None,
}

impl AppSource {
    pub fn infer_source(source: &str) -> Self {
        let path = PathBuf::from(source);
        if path.exists() {
            Self::infer_file_source(path)
        } else if spin_oci::is_probably_oci_reference(source) {
            Self::OciRegistry(source.to_owned())
        } else {
            Self::Unresolvable(format!("File or directory '{source}' not found. If you meant to load from a registry, use the `--from-registry` option."))
        }
    }

    pub fn infer_file_source(path: impl Into<PathBuf>) -> Self {
        match spin_common::paths::resolve_manifest_file_path(path.into()) {
            Ok(file) => {
                if is_wasm_file(&file) {
                    Self::BareWasm(file)
                } else {
                    Self::File(file)
                }
            }
            Err(e) => Self::Unresolvable(e.to_string()),
        }
    }

    pub fn unresolvable(message: impl Into<String>) -> Self {
        Self::Unresolvable(message.into())
    }

    pub fn local_app_dir(&self) -> Option<&Path> {
        match self {
            Self::File(path) => path.parent().or_else(|| {
                tracing::warn!("Error finding local app dir from source {path:?}");
                None
            }),
            _ => None,
        }
    }

    pub async fn build(&self, cache_root: &Option<PathBuf>) -> anyhow::Result<()> {
        match self {
            Self::File(path) => spin_build::build_default(path, cache_root.clone()).await,
            _ => Ok(()),
        }
    }
}

fn is_wasm_file(path: &Path) -> bool {
    let extn = path.extension().and_then(std::ffi::OsStr::to_str);
    extn.is_some_and(|e| e == "wasm" || e == "wat")
}

impl std::fmt::Display for AppSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(path) => write!(f, "local app {}", quoted_path(path)),
            Self::OciRegistry(reference) => write!(f, "remote app {reference:?}"),
            Self::BareWasm(path) => write!(f, "Wasm file {}", quoted_path(path)),
            Self::Unresolvable(s) => write!(f, "unknown app source: {s:?}"),
            Self::None => write!(f, "<no source>"),
        }
    }
}

/// This represents a "partially loaded" source which has enough information to
/// dispatch to the correct trigger executor but hasn't (necessarily) gone
/// through full validation / loading yet.
#[allow(clippy::large_enum_variant)] // allow because the large variant is the common one
pub enum ResolvedAppSource {
    File {
        manifest_path: PathBuf,
        manifest: AppManifest,
    },
    BareWasm {
        wasm_path: PathBuf,
    },
    OciRegistry {
        locked_app: LockedApp,
    },
}

impl ResolvedAppSource {
    pub fn trigger_types(&self) -> Vec<&str> {
        let types = match self {
            ResolvedAppSource::File { manifest, .. } => manifest
                .triggers
                .keys()
                .map(|s| s.as_str())
                .collect::<HashSet<_>>(),
            ResolvedAppSource::OciRegistry { locked_app } => locked_app
                .triggers
                .iter()
                .map(|t| t.trigger_type.as_str())
                .collect::<HashSet<_>>(),
            ResolvedAppSource::BareWasm { .. } => ["http"].into_iter().collect(),
        };

        types.into_iter().collect()
    }
}
