mod store;

use std::{fs, path::PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use spin_factor_key_value::runtime_config::spin::MakeKeyValueStore;
use store::{DatabaseLocation, KeyValueSqlite};

/// A key-value store that uses SQLite as the backend.
pub struct SpinKeyValueStore {
    /// The base path or directory for the SQLite database file.
    base_path: Option<PathBuf>,
}

impl SpinKeyValueStore {
    /// Create a new SpinKeyValueStore with the given base path.
    ///
    /// If the database directory is None, the database will always be in-memory.
    /// If it's `Some`, the database will be stored at the combined `base_path` and
    /// the `path` specified in the runtime configuration.
    pub fn new(base_path: Option<PathBuf>) -> Self {
        Self { base_path }
    }
}

impl MakeKeyValueStore for SpinKeyValueStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "spin";

    type RuntimeConfig = SpinKeyValueRuntimeConfig;

    type StoreManager = KeyValueSqlite;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::StoreManager> {
        let location = match (&self.base_path, &runtime_config.path) {
            // If both the base path and the path are specified, resolve the path against the base path
            (Some(base_path), Some(path)) => DatabaseLocation::Path(base_path.join(path)),
            // If the base path is `None` but use the path without resolving relative to the base path.
            (None, Some(path)) => DatabaseLocation::Path(path.clone()),
            // Otherwise, use an in-memory database
            (None | Some(_), None) => DatabaseLocation::InMemory,
        };
        if let DatabaseLocation::Path(path) = &location {
            // Create the store's parent directory if necessary
            if let Some(parent) = path.parent().filter(|p| !p.exists()) {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create key value store's parent directory: '{}",
                        parent.display()
                    )
                })?;
            }
        }
        Ok(KeyValueSqlite::new(location))
    }
}

/// The serialized runtime configuration for the SQLite key-value store.
#[derive(Deserialize, Serialize)]
pub struct SpinKeyValueRuntimeConfig {
    /// The path to the SQLite database file.
    path: Option<PathBuf>,
}

impl SpinKeyValueRuntimeConfig {
    /// Create a new SpinKeyValueRuntimeConfig with the given parent directory
    /// where the key-value store will live.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}
