//! Resolves Spin's default data directory paths

use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// Return the default data directory for Spin
pub fn data_dir() -> Result<PathBuf> {
    if let Ok(data_dir) = std::env::var("SPIN_DATA_DIR") {
        return Ok(PathBuf::from(data_dir));
    }
    if let Some(pkg_mgr_dir) = package_manager_data_dir() {
        return Ok(pkg_mgr_dir);
    }

    let data_dir = dirs::data_local_dir()
        .or_else(|| dirs::home_dir().map(|p| p.join(".spin")))
        .ok_or_else(|| anyhow!("Unable to get local data directory or home directory"))?;
    Ok(data_dir.join("spin"))
}

/// Get the package manager specific data directory
fn package_manager_data_dir() -> Option<PathBuf> {
    if let Ok(brew_prefix) = std::env::var("HOMEBREW_PREFIX")
        && std::env::current_exe()
            .map(|p| p.starts_with(&brew_prefix))
            .unwrap_or(false)
    {
        let data_dir = Path::new(&brew_prefix)
            .join("etc")
            .join("spinframework-spin");
        return Some(data_dir);
    }
    None
}
