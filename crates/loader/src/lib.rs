//! Loaders for Spin applications.
//! This crate implements the possible application sources for Spin applications,
//! and includes functionality to convert the specific configuration (for example
//! local configuration files or from OCI) into Spin configuration that
//! can be consumed by the Spin execution context.
//!
//! This crate can be extended (or replaced entirely) to support additional loaders,
//! and any implementation that produces a `Application` is compatible
//! with the Spin execution context.

#![deny(missing_docs)]

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use local::LocalLoader;
use spin_app::locked::LockedApp;
use spin_common::paths::parent_dir;

pub mod cache;
mod fs;
#[cfg(feature = "async-io")]
mod http;
mod local;

pub use local::requires_service_chaining;
pub use local::WasmLoader;

/// Maximum number of files to copy (or download) concurrently
pub(crate) const MAX_FILE_LOADING_CONCURRENCY: usize = 16;

/// Load a Spin locked app from a spin.toml manifest file. If `files_mount_root`
/// is given, `files` mounts will be copied to that directory. If not, `files`
/// mounts will validated as "direct mounts".
pub async fn from_file(
    manifest_path: impl AsRef<Path>,
    files_mount_strategy: FilesMountStrategy,
    cache_root: Option<PathBuf>,
) -> Result<LockedApp> {
    let path = manifest_path.as_ref();
    let app_root = parent_dir(path).context("manifest path has no parent directory")?;
    let loader = LocalLoader::new(&app_root, files_mount_strategy, cache_root).await?;
    loader.load_file(path).await
}

/// Load a Spin locked app from a standalone Wasm file.
pub async fn from_wasm_file(wasm_path: impl AsRef<Path>) -> Result<LockedApp> {
    let app_root = std::env::current_dir()?;
    let manifest = single_file_manifest(wasm_path)?;
    let loader = LocalLoader::new(&app_root, FilesMountStrategy::Direct, None).await?;
    loader.load_manifest(manifest).await
}

/// The strategy to use for mounting WASI files into a guest.
#[derive(Debug)]
pub enum FilesMountStrategy {
    /// Copy files into the given mount root directory.
    Copy(PathBuf),
    /// Mount files directly from their source director(ies). This only
    /// supports mounting full directories; mounting single files, glob
    /// patterns, and `exclude_files` are not supported.
    Direct,
}

fn single_file_manifest(
    wasm_path: impl AsRef<Path>,
) -> anyhow::Result<spin_manifest::schema::v2::AppManifest> {
    use serde::Deserialize;

    let wasm_path_str = wasm_path
        .as_ref()
        .to_str()
        .context("Failed to stringise Wasm file path")?
        .to_owned();
    let app_name = wasm_path
        .as_ref()
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wasm-file")
        .to_owned();

    let manifest = toml::toml!(
        spin_manifest_version = 2

        [application]
        name = app_name

        [[trigger.http]]
        route = "/..."
        component = { source = wasm_path_str }
    );

    let manifest = spin_manifest::schema::v2::AppManifest::deserialize(manifest)?;

    Ok(manifest)
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use spin_app::{retain_components, App};

    use super::*;

    pub async fn build_locked_app(manifest: &toml::Table) -> anyhow::Result<LockedApp> {
        let toml_str = toml::to_string(manifest).context("failed serializing manifest")?;
        let dir = tempfile::tempdir().context("failed creating tempdir")?;
        let path = dir.path().join("spin.toml");
        std::fs::write(&path, toml_str).context("failed writing manifest")?;
        from_file(&path, FilesMountStrategy::Direct, None).await
    }

    fn does_nothing_validator(_: &App, _: &[&str]) -> anyhow::Result<()> {
        Ok(())
    }

    #[tokio::test]
    async fn test_retain_components_filtering_for_only_component_works() {
        let manifest = toml::toml! {
            spin_manifest_version = 2

            [application]
            name = "test-app"

            [[trigger.test-trigger]]
            component = "empty"

            [component.empty]
            source = "does-not-exist.wasm"
        };
        let mut locked_app = build_locked_app(&manifest).await.unwrap();
        locked_app = retain_components(locked_app, &["empty"], &[&does_nothing_validator]).unwrap();
        let components = locked_app
            .components
            .iter()
            .map(|c| c.id.to_string())
            .collect::<HashSet<_>>();
        assert!(components.contains("empty"));
        assert!(components.len() == 1);
    }
}
