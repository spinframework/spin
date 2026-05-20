use anyhow::Result;
use flate2::read::GzDecoder;
use spin_common::data_dir::data_dir;
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};
use tar::Archive;

use crate::{Catalogue, manifest::PluginManifest};

/// Directory where the manifests of installed plugins are stored.
pub const PLUGIN_MANIFESTS_DIRECTORY_NAME: &str = "manifests";
const INSTALLATION_RECORD_FILE_NAME: &str = ".install.json";

// Name of directory that contains the cloned centralized Spin plugins
// repository
const LOCAL_SNAPSHOT_PATH_IN_STORE: &str = ".spin-plugins";

/// Houses utilities for getting the path to Spin plugin directories.
pub struct PluginStore {
    root: PathBuf,
}

impl PluginStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn try_default() -> Result<Self> {
        Ok(Self::new(data_dir()?.join("plugins")))
    }

    /// Gets the path to where Spin plugin are installed.
    pub fn get_plugins_directory(&self) -> &Path {
        &self.root
    }

    /// Get the path to the subdirectory of an installed plugin.
    pub fn plugin_subdirectory_path(&self, plugin_name: &str) -> PathBuf {
        self.root.join(plugin_name)
    }

    pub fn installed_binary_path(&self, plugin_name: &str) -> PathBuf {
        let mut binary = self.root.join(plugin_name).join(plugin_name);
        if cfg!(target_os = "windows") {
            binary.set_extension("exe");
        }
        binary
    }

    /// Get the path to the manifests directory which contains the plugin manifests
    /// of all installed Spin plugins.
    pub fn installed_manifests_directory(&self) -> PathBuf {
        self.root.join(PLUGIN_MANIFESTS_DIRECTORY_NAME)
    }

    pub fn installed_manifest_path(&self, plugin_name: &str) -> PathBuf {
        self.installed_manifests_directory()
            .join(crate::util::manifest_file_name(plugin_name))
    }

    pub fn installation_record_file(&self, plugin_name: &str) -> PathBuf {
        self.root
            .join(plugin_name)
            .join(INSTALLATION_RECORD_FILE_NAME)
    }

    pub(crate) fn catalogue(&self) -> Catalogue {
        let git_root = self.root.join(LOCAL_SNAPSHOT_PATH_IN_STORE);
        Catalogue::new(git_root)
    }

    pub(crate) fn add_manifest(&self, plugin_manifest: &PluginManifest) -> Result<()> {
        let manifests_dir = self.installed_manifests_directory();
        std::fs::create_dir_all(manifests_dir)?;
        serde_json::to_writer(
            &File::create(self.installed_manifest_path(&plugin_manifest.name()))?,
            plugin_manifest,
        )?;
        tracing::trace!("Added manifest for {}", &plugin_manifest.name());
        Ok(())
    }

    pub(crate) fn untar_plugin(&self, plugin_file_name: &PathBuf, plugin_name: &str) -> Result<()> {
        // Get handle to file
        let tar_gz = File::open(plugin_file_name)?;
        // Unzip file
        let tar = GzDecoder::new(tar_gz);
        // Get plugin from tarball
        let mut archive = Archive::new(tar);
        archive.set_preserve_permissions(true);
        // Create subdirectory in plugins directory for this plugin
        let plugin_sub_dir = self.plugin_subdirectory_path(plugin_name);
        fs::remove_dir_all(&plugin_sub_dir).ok();
        fs::create_dir_all(&plugin_sub_dir)?;
        archive.unpack(&plugin_sub_dir)?;
        Ok(())
    }
}
