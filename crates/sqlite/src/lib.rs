//! Spin's default handling of the runtime configuration for SQLite databases.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Deserialize;
use spin_factor_sqlite::ConnectionCreator;
use spin_factors::{
    anyhow::{self, Context as _},
    runtime_config::toml::GetTomlValue,
};
use spin_sqlite_inproc::InProcDatabaseLocation;
use spin_sqlite_libsql::LazyLibSqlConnection;

/// Spin's default resolution of runtime configuration for SQLite databases.
///
/// This type implements how Spin CLI's SQLite implementation is configured
/// through the runtime config toml as well as the behavior of the "default" label.
#[derive(Clone, Debug)]
pub struct RuntimeConfigResolver {
    default_database_dir: Option<PathBuf>,
    local_database_dir: PathBuf,
}

impl RuntimeConfigResolver {
    /// Create a new `SpinSqliteRuntimeConfig`
    ///
    /// This takes as arguments:
    /// * the directory to use as the default location for SQLite databases.
    ///   Usually this will be the path to the `.spin` state directory. If
    ///   `None`, the default database will be in-memory.
    /// * the path to the directory from which relative paths to
    ///   local SQLite databases are resolved.  (this should most likely be the
    ///   path to the runtime-config file or the current working dir).
    pub fn new(default_database_dir: Option<PathBuf>, local_database_dir: PathBuf) -> Self {
        Self {
            default_database_dir,
            local_database_dir,
        }
    }

    /// Get the runtime configuration for SQLite databases from a TOML table.
    ///
    /// Expects table to be in the format:
    /// ````toml
    /// [sqlite_database.$database-label]
    /// type = "$database-type"
    /// ... extra type specific configuration ...
    /// ```
    ///
    /// Configuration is automatically added for the 'default' label if it is not provided.
    pub fn resolve(
        &self,
        table: &impl GetTomlValue,
    ) -> anyhow::Result<spin_factor_sqlite::runtime_config::RuntimeConfig> {
        let mut runtime_config = self.resolve_from_toml(table)?.unwrap_or_default();
        // If the user did not provide configuration for the default label, add it.
        if !runtime_config.connection_creators.contains_key("default") {
            runtime_config
                .connection_creators
                .insert("default".to_owned(), self.default());
        }

        Ok(runtime_config)
    }

    /// Get the runtime configuration for SQLite databases from a TOML table.
    fn resolve_from_toml(
        &self,
        table: &impl GetTomlValue,
    ) -> anyhow::Result<Option<spin_factor_sqlite::runtime_config::RuntimeConfig>> {
        let Some(table) = table.get("sqlite_database") else {
            return Ok(None);
        };
        let config: std::collections::HashMap<String, TomlRuntimeConfig> =
            table.clone().try_into()?;
        let connection_creators = config
            .into_iter()
            .map(|(k, v)| Ok((k, self.get_connection_creator(v)?)))
            .collect::<anyhow::Result<HashMap<_, _>>>()?;

        Ok(Some(spin_factor_sqlite::runtime_config::RuntimeConfig {
            connection_creators,
        }))
    }

    /// Get a connection creator for a given runtime configuration.
    pub fn get_connection_creator(
        &self,
        config: TomlRuntimeConfig,
    ) -> anyhow::Result<Arc<dyn ConnectionCreator>> {
        let database_kind = config.type_.as_str();
        match database_kind {
            "spin" => {
                let config: InProcDatabase = config.config.try_into()?;
                Ok(Arc::new(
                    config.connection_creator(&self.local_database_dir)?,
                ))
            }
            "libsql" => {
                let config: LibSqlDatabase = config.config.try_into()?;
                Ok(Arc::new(config.connection_creator()?))
            }
            _ => anyhow::bail!("Unknown database kind: {database_kind}"),
        }
    }
}

#[derive(Deserialize)]
pub struct TomlRuntimeConfig {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(flatten)]
    pub config: toml::Table,
}

impl RuntimeConfigResolver {
    /// The [`ConnectionCreator`] for the 'default' label.
    pub fn default(&self) -> Arc<dyn ConnectionCreator> {
        let path = self
            .default_database_dir
            .as_deref()
            .map(|p| p.join(DEFAULT_SQLITE_DB_FILENAME));
        let factory = move || {
            let location = InProcDatabaseLocation::from_path(path.clone())?;
            let connection = spin_sqlite_inproc::InProcConnection::new(location)?;
            Ok(Arc::new(connection) as _)
        };
        Arc::new(factory)
    }
}

const DEFAULT_SQLITE_DB_FILENAME: &str = "sqlite_db.db";

/// Configuration for a local SQLite database.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InProcDatabase {
    pub path: Option<PathBuf>,
}

impl InProcDatabase {
    /// Get a new connection creator for a local database.
    ///
    /// `base_dir` is the base directory path from which `path` is resolved if it is a relative path.
    fn connection_creator(self, base_dir: &Path) -> anyhow::Result<impl ConnectionCreator> {
        let path = self
            .path
            .as_ref()
            .map(|p| resolve_relative_path(p, base_dir));
        let location = InProcDatabaseLocation::from_path(path)?;
        let factory = move || {
            let connection = spin_sqlite_inproc::InProcConnection::new(location.clone())?;
            Ok(Arc::new(connection) as _)
        };
        Ok(factory)
    }
}

/// Resolve a relative path against a base dir.
///
/// If the path is absolute, it is returned as is. Otherwise, it is resolved against the base dir.
fn resolve_relative_path(path: &Path, base_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }
    base_dir.join(path)
}

/// Configuration for a libSQL database.
///
/// This is used to deserialize the specific runtime config toml for libSQL databases.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibSqlDatabase {
    url: String,
    token: String,
}

impl LibSqlDatabase {
    /// Get a new connection creator for a libSQL database.
    fn connection_creator(self) -> anyhow::Result<impl ConnectionCreator> {
        let url = check_url(&self.url)
            .with_context(|| {
                format!(
                    "unexpected libSQL URL '{}' in runtime config file ",
                    self.url
                )
            })?
            .to_owned();
        let factory = move || {
            let connection = LazyLibSqlConnection::new(url.clone(), self.token.clone());
            Ok(Arc::new(connection) as _)
        };
        Ok(factory)
    }
}

// Checks an incoming url is in the shape we expect
fn check_url(url: &str) -> anyhow::Result<&str> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(url)
    } else {
        Err(anyhow::anyhow!(
            "URL does not start with 'https://' or 'http://'. Spin currently only supports talking to libSQL databases over HTTP(S)"
        ))
    }
}
