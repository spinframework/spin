use crate::manifest::PluginManifest;
use std::{
    ffi::OsStr,
    fs::File,
    path::{Path, PathBuf},
};

pub fn try_read_manifest_from(manifest_path: &Path) -> Option<PluginManifest> {
    let manifest_file = File::open(manifest_path).ok()?;
    serde_json::from_reader(manifest_file).ok()
}

pub fn json_files_in(dir: &Path) -> Vec<PathBuf> {
    let json_ext = Some(OsStr::new("json"));
    match dir.read_dir() {
        Err(_) => vec![],
        Ok(rd) => rd
            .filter_map(|de| de.ok())
            .map(|de| de.path())
            .filter(|p| p.is_file() && p.extension() == json_ext)
            .collect(),
    }
}

// Given a name and option version, outputs expected file name for the plugin.
pub fn manifest_file_name_version(plugin_name: &str, version: &Option<semver::Version>) -> String {
    match version {
        Some(v) => format!("{plugin_name}@{v}.json"),
        None => manifest_file_name(plugin_name),
    }
}

/// Given a plugin name, returns the expected file name for the installed manifest
pub fn manifest_file_name(plugin_name: &str) -> String {
    format!("{plugin_name}.json")
}
