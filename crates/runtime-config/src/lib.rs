use std::path::{Path, PathBuf};

use anyhow::Context as _;
use spin_common::ui::quoted_path;
use spin_factor_key_value::runtime_config::spin::{self as key_value};
use spin_factor_key_value::KeyValueFactor;
use spin_factor_llm::{spin as llm, LlmFactor};
use spin_factor_outbound_http::OutboundHttpFactor;
use spin_factor_outbound_mqtt::OutboundMqttFactor;
use spin_factor_outbound_mysql::OutboundMysqlFactor;
use spin_factor_outbound_networking::runtime_config::spin::SpinRuntimeConfig as OutboundNetworkingSpinRuntimeConfig;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_outbound_pg::OutboundPgFactor;
use spin_factor_outbound_redis::OutboundRedisFactor;
use spin_factor_sqlite::SqliteFactor;
use spin_factor_variables::VariablesFactor;
use spin_factor_wasi::WasiFactor;
use spin_factors::runtime_config::toml::GetTomlValue as _;
use spin_factors::{
    runtime_config::toml::TomlKeyTracker, FactorRuntimeConfigSource, RuntimeConfigSourceFinalizer,
};
use spin_key_value_spin::{SpinKeyValueRuntimeConfig, SpinKeyValueStore};
use spin_sqlite as sqlite;
use spin_trigger::cli::UserProvidedPath;
use toml::Value;

pub mod variables;

/// The default state directory for the trigger.
pub const DEFAULT_STATE_DIR: &str = ".spin";

/// A runtime configuration which has been resolved from a runtime config source.
///
/// Includes other pieces of configuration that are used to resolve the runtime configuration.
pub struct ResolvedRuntimeConfig<T> {
    /// The resolved runtime configuration.
    pub runtime_config: T,
    /// The resolver used to resolve key-value stores from runtime configuration.
    pub key_value_resolver: key_value::RuntimeConfigResolver,
    /// The resolver used to resolve sqlite databases from runtime configuration.
    pub sqlite_resolver: sqlite::RuntimeConfigResolver,
    /// The fully resolved state directory.
    ///
    /// `None` is used for an "unset" state directory which each factor will treat differently.
    pub state_dir: Option<PathBuf>,
    /// The fully resolved log directory.
    ///
    /// `None` is used for an "unset" log directory.
    pub log_dir: Option<PathBuf>,
    /// The maximum memory allocation limit.
    pub max_instance_memory: Option<usize>,
    /// The input TOML, for informational summaries.
    pub toml: toml::Table,
}

impl<T> ResolvedRuntimeConfig<T> {
    pub fn summarize(&self, runtime_config_path: Option<&Path>) {
        let summarize_labeled_typed_tables = |key| {
            let mut summaries = vec![];
            if let Some(tables) = self.toml.get(key).and_then(Value::as_table) {
                for (label, config) in tables {
                    if let Some(ty) = config.get("type").and_then(Value::as_str) {
                        summaries.push(format!("[{key}.{label}: {ty}]"))
                    }
                }
            }
            summaries
        };

        let mut summaries = vec![];
        // [key_value_store.<label>: <type>]
        summaries.extend(summarize_labeled_typed_tables("key_value_store"));
        // [sqlite_database.<label>: <type>]
        summaries.extend(summarize_labeled_typed_tables("sqlite_database"));
        // [llm_compute: <type>]
        if let Some(table) = self.toml.get("llm_compute").and_then(Value::as_table) {
            if let Some(ty) = table.get("type").and_then(Value::as_str) {
                summaries.push(format!("[llm_compute: {ty}"));
            }
        }
        if !summaries.is_empty() {
            let summaries = summaries.join(", ");
            let from_path = runtime_config_path
                .map(|path| format!("from {}", quoted_path(path)))
                .unwrap_or_default();
            println!("Using runtime config {summaries} {from_path}");
        }
    }
}

impl<T> ResolvedRuntimeConfig<T>
where
    T: for<'a, 'b> TryFrom<TomlRuntimeConfigSource<'a, 'b>>,
    for<'a, 'b> <T as TryFrom<TomlRuntimeConfigSource<'a, 'b>>>::Error: Into<anyhow::Error>,
{
    /// Creates a new resolved runtime configuration from a runtime config source TOML file.
    ///
    /// `provided_state_dir` is the explicitly provided state directory, if any.
    pub fn from_file(
        runtime_config_path: Option<&Path>,
        local_app_dir: Option<PathBuf>,
        provided_state_dir: UserProvidedPath,
        provided_log_dir: UserProvidedPath,
    ) -> anyhow::Result<Self> {
        let toml = match runtime_config_path {
            Some(runtime_config_path) => {
                let file = std::fs::read_to_string(runtime_config_path).with_context(|| {
                    format!(
                        "failed to read runtime config file '{}'",
                        runtime_config_path.display()
                    )
                })?;
                toml::from_str(&file).with_context(|| {
                    format!(
                        "failed to parse runtime config file '{}' as toml",
                        runtime_config_path.display()
                    )
                })?
            }
            None => Default::default(),
        };
        let toml_resolver =
            TomlResolver::new(&toml, local_app_dir, provided_state_dir, provided_log_dir);

        Self::new(toml_resolver, runtime_config_path)
    }

    /// Creates a new resolved runtime configuration from a TOML table.
    pub fn new(
        toml_resolver: TomlResolver<'_>,
        runtime_config_path: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let runtime_config_dir = runtime_config_path
            .and_then(Path::parent)
            .map(ToOwned::to_owned);
        let state_dir = toml_resolver.state_dir()?;
        let outbound_networking = runtime_config_dir
            .clone()
            .map(OutboundNetworkingSpinRuntimeConfig::new);
        let key_value_resolver = key_value_config_resolver(runtime_config_dir, state_dir.clone());
        let sqlite_resolver = sqlite_config_resolver(state_dir.clone())
            .context("failed to resolve sqlite runtime config")?;

        let toml = toml_resolver.toml();
        let log_dir = toml_resolver.log_dir()?;
        let max_instance_memory = toml_resolver.max_instance_memory()?;

        let source = TomlRuntimeConfigSource::new(
            toml_resolver,
            &key_value_resolver,
            outbound_networking.as_ref(),
            &sqlite_resolver,
        );

        // Note: all valid fields in the runtime config must have been referenced at
        // this point or the finalizer will fail due to `validate_all_keys_used`
        // not passing.
        let runtime_config: T = source.try_into().map_err(Into::into)?;

        Ok(Self {
            runtime_config,
            key_value_resolver,
            sqlite_resolver,
            state_dir,
            log_dir,
            max_instance_memory,
            toml,
        })
    }

    /// The fully resolved state directory.
    pub fn state_dir(&self) -> Option<PathBuf> {
        self.state_dir.clone()
    }

    /// The fully resolved state directory.
    pub fn log_dir(&self) -> Option<PathBuf> {
        self.log_dir.clone()
    }

    /// The maximum memory allocation limit.
    pub fn max_instance_memory(&self) -> Option<usize> {
        self.max_instance_memory
    }
}

#[derive(Clone, Debug)]
/// Resolves runtime configuration from a TOML file.
pub struct TomlResolver<'a> {
    table: TomlKeyTracker<'a>,
    /// The local app directory.
    local_app_dir: Option<PathBuf>,
    /// Explicitly provided state directory.
    state_dir: UserProvidedPath,
    /// Explicitly provided log directory.
    log_dir: UserProvidedPath,
}

impl<'a> TomlResolver<'a> {
    /// Create a new TOML resolver.
    pub fn new(
        table: &'a toml::Table,
        local_app_dir: Option<PathBuf>,
        state_dir: UserProvidedPath,
        log_dir: UserProvidedPath,
    ) -> Self {
        Self {
            table: TomlKeyTracker::new(table),
            local_app_dir,
            state_dir,
            log_dir,
        }
    }

    /// Get the configured state_directory.
    ///
    /// Errors if the path cannot be converted to an absolute path.
    pub fn state_dir(&self) -> std::io::Result<Option<PathBuf>> {
        let mut state_dir = self.state_dir.clone();
        // If the state_dir is not explicitly provided, check the toml.
        if matches!(state_dir, UserProvidedPath::Default) {
            let from_toml =
                self.table
                    .get("state_dir")
                    .and_then(|v| v.as_str())
                    .map(|toml_value| {
                        if toml_value.is_empty() {
                            // If the toml value is empty, treat it as unset.
                            UserProvidedPath::Unset
                        } else {
                            // Otherwise, treat the toml value as a provided path.
                            UserProvidedPath::Provided(PathBuf::from(toml_value))
                        }
                    });
            // If toml value is not provided, use the original value after all.
            state_dir = from_toml.unwrap_or(state_dir);
        }

        match (state_dir, &self.local_app_dir) {
            (UserProvidedPath::Provided(p), _) => Ok(Some(std::path::absolute(p)?)),
            (UserProvidedPath::Default, Some(local_app_dir)) => {
                Ok(Some(local_app_dir.join(".spin")))
            }
            (UserProvidedPath::Default | UserProvidedPath::Unset, _) => Ok(None),
        }
    }

    /// Get the configured log directory.
    ///
    /// Errors if the path cannot be converted to an absolute path.
    pub fn log_dir(&self) -> std::io::Result<Option<PathBuf>> {
        let mut log_dir = self.log_dir.clone();
        // If the log_dir is not explicitly provided, check the toml.
        if matches!(log_dir, UserProvidedPath::Default) {
            let from_toml = self
                .table
                .get("log_dir")
                .and_then(|v| v.as_str())
                .map(|toml_value| {
                    if toml_value.is_empty() {
                        // If the toml value is empty, treat it as unset.
                        UserProvidedPath::Unset
                    } else {
                        // Otherwise, treat the toml value as a provided path.
                        UserProvidedPath::Provided(PathBuf::from(toml_value))
                    }
                });
            // If toml value is not provided, use the original value after all.
            log_dir = from_toml.unwrap_or(log_dir);
        }

        match log_dir {
            UserProvidedPath::Provided(p) => Ok(Some(std::path::absolute(p)?)),
            UserProvidedPath::Default => Ok(self.state_dir()?.map(|p| p.join("logs"))),
            UserProvidedPath::Unset => Ok(None),
        }
    }

    /// Get the configured maximum memory allocation limit.
    pub fn max_instance_memory(&self) -> anyhow::Result<Option<usize>> {
        self.table
            .get("max_instance_memory")
            .and_then(|v| v.as_integer())
            .map(|toml_value| toml_value.try_into())
            .transpose()
            .map_err(Into::into)
    }

    /// Validate that all keys in the TOML file have been used.
    pub fn validate_all_keys_used(&self) -> spin_factors::Result<()> {
        self.table.validate_all_keys_used()
    }

    fn toml(&self) -> toml::Table {
        self.table.as_ref().clone()
    }
}

/// The TOML based runtime configuration source Spin CLI.
pub struct TomlRuntimeConfigSource<'a, 'b> {
    toml: TomlResolver<'b>,
    key_value: &'a key_value::RuntimeConfigResolver,
    outbound_networking: Option<&'a OutboundNetworkingSpinRuntimeConfig>,
    sqlite: &'a sqlite::RuntimeConfigResolver,
}

impl<'a, 'b> TomlRuntimeConfigSource<'a, 'b> {
    pub fn new(
        toml_resolver: TomlResolver<'b>,
        key_value: &'a key_value::RuntimeConfigResolver,
        outbound_networking: Option<&'a OutboundNetworkingSpinRuntimeConfig>,
        sqlite: &'a sqlite::RuntimeConfigResolver,
    ) -> Self {
        Self {
            toml: toml_resolver,
            key_value,
            outbound_networking,
            sqlite,
        }
    }
}

impl FactorRuntimeConfigSource<KeyValueFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(
        &mut self,
    ) -> anyhow::Result<Option<spin_factor_key_value::RuntimeConfig>> {
        Ok(Some(self.key_value.resolve(Some(&self.toml.table))?))
    }
}

impl FactorRuntimeConfigSource<OutboundNetworkingFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(
        &mut self,
    ) -> anyhow::Result<Option<<OutboundNetworkingFactor as spin_factors::Factor>::RuntimeConfig>>
    {
        let Some(tls) = self.outbound_networking else {
            return Ok(None);
        };
        tls.config_from_table(&self.toml.table)
    }
}

impl FactorRuntimeConfigSource<VariablesFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(
        &mut self,
    ) -> anyhow::Result<Option<<VariablesFactor as spin_factors::Factor>::RuntimeConfig>> {
        Ok(Some(variables::runtime_config_from_toml(&self.toml.table)?))
    }
}

impl FactorRuntimeConfigSource<OutboundPgFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<()>> {
        Ok(None)
    }
}

impl FactorRuntimeConfigSource<OutboundMysqlFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<()>> {
        Ok(None)
    }
}

impl FactorRuntimeConfigSource<LlmFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<spin_factor_llm::RuntimeConfig>> {
        llm::runtime_config_from_toml(&self.toml.table, self.toml.state_dir()?)
    }
}

impl FactorRuntimeConfigSource<OutboundRedisFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<()>> {
        Ok(None)
    }
}

impl FactorRuntimeConfigSource<WasiFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<()>> {
        Ok(None)
    }
}

impl FactorRuntimeConfigSource<OutboundHttpFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<()>> {
        Ok(None)
    }
}

impl FactorRuntimeConfigSource<OutboundMqttFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<()>> {
        Ok(None)
    }
}

impl FactorRuntimeConfigSource<SqliteFactor> for TomlRuntimeConfigSource<'_, '_> {
    fn get_runtime_config(&mut self) -> anyhow::Result<Option<spin_factor_sqlite::RuntimeConfig>> {
        Ok(Some(self.sqlite.resolve(&self.toml.table)?))
    }
}

impl RuntimeConfigSourceFinalizer for TomlRuntimeConfigSource<'_, '_> {
    fn finalize(&mut self) -> anyhow::Result<()> {
        Ok(self.toml.validate_all_keys_used()?)
    }
}

const DEFAULT_KEY_VALUE_STORE_LABEL: &str = "default";

/// The key-value runtime configuration resolver.
///
/// Takes a base path that all local key-value stores which are configured with
/// relative paths will be relative to. It also takes a default store base path
/// which will be used as the directory for the default store.
pub fn key_value_config_resolver(
    local_store_base_path: Option<PathBuf>,
    default_store_base_path: Option<PathBuf>,
) -> key_value::RuntimeConfigResolver {
    let mut key_value = key_value::RuntimeConfigResolver::new();

    // Register the supported store types.
    // Unwraps are safe because the store types are known to not overlap.
    key_value
        .register_store_type(spin_key_value_spin::SpinKeyValueStore::new(
            local_store_base_path.clone(),
        ))
        .unwrap();
    key_value
        .register_store_type(spin_key_value_redis::RedisKeyValueStore::new())
        .unwrap();
    key_value
        .register_store_type(spin_key_value_azure::AzureKeyValueStore::new(None))
        .unwrap();
    key_value
        .register_store_type(spin_key_value_aws::AwsDynamoKeyValueStore::new())
        .unwrap();

    // Add handling of "default" store.
    let default_store_path = default_store_base_path.map(|p| p.join(DEFAULT_SPIN_STORE_FILENAME));
    // Unwraps are safe because the store is known to be serializable as toml.
    key_value
        .add_default_store::<SpinKeyValueStore>(
            DEFAULT_KEY_VALUE_STORE_LABEL,
            SpinKeyValueRuntimeConfig::new(default_store_path),
        )
        .unwrap();

    key_value
}

/// The default filename for the SQLite database.
const DEFAULT_SPIN_STORE_FILENAME: &str = "sqlite_key_value.db";

/// The sqlite runtime configuration resolver.
///
/// Takes a path to the directory where the default database should be stored.
/// If the path is `None`, the default database will be in-memory.
fn sqlite_config_resolver(
    default_database_dir: Option<PathBuf>,
) -> anyhow::Result<sqlite::RuntimeConfigResolver> {
    let local_database_dir =
        std::env::current_dir().context("failed to get current working directory")?;
    Ok(sqlite::RuntimeConfigResolver::new(
        default_database_dir,
        local_database_dir,
    ))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use spin_factors::RuntimeFactors;
    use spin_factors_test::TestEnvironment;

    use super::*;

    /// Define a test factor with the given field and factor type.
    macro_rules! define_test_factor {
        ($field:ident : $factor:ty) => {
            #[derive(RuntimeFactors)]
            struct TestFactors {
                $field: $factor,
            }
            impl TryFrom<TomlRuntimeConfigSource<'_, '_>> for TestFactorsRuntimeConfig {
                type Error = anyhow::Error;

                fn try_from(value: TomlRuntimeConfigSource<'_, '_>) -> Result<Self, Self::Error> {
                    Self::from_source(value)
                }
            }
            fn resolve_toml(
                toml: toml::Table,
                path: impl AsRef<std::path::Path>,
            ) -> anyhow::Result<ResolvedRuntimeConfig<TestFactorsRuntimeConfig>> {
                ResolvedRuntimeConfig::<TestFactorsRuntimeConfig>::new(
                    toml_resolver(&toml),
                    Some(path.as_ref()),
                )
            }
        };
    }

    #[test]
    fn sqlite_is_configured_correctly() {
        define_test_factor!(sqlite: SqliteFactor);

        impl TestFactorsRuntimeConfig {
            /// Get the connection creators for the configured sqlite databases.
            fn connection_creators(
                &self,
            ) -> &HashMap<String, Arc<dyn spin_factor_sqlite::ConnectionCreator>> {
                &self.sqlite.as_ref().unwrap().connection_creators
            }

            /// Get the labels of the configured sqlite databases.
            fn configured_labels(&self) -> Vec<&str> {
                let mut configured_labels = self
                    .connection_creators()
                    .keys()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>();
                // Sort the labels to ensure consistent ordering.
                configured_labels.sort();
                configured_labels
            }
        }

        // Test that the default label is added if not provided.
        let toml = toml::toml! {
            [sqlite_database.foo]
            type = "spin"
        };
        assert_eq!(
            resolve_toml(toml, ".")
                .unwrap()
                .runtime_config
                .configured_labels(),
            vec!["default", "foo"]
        );

        // Test that the default label is added with an empty toml config.
        let toml = toml::Table::new();
        let runtime_config = resolve_toml(toml, "config.toml").unwrap().runtime_config;
        assert_eq!(runtime_config.configured_labels(), vec!["default"]);
    }

    #[test]
    fn key_value_is_configured_correctly() {
        define_test_factor!(key_value: KeyValueFactor);
        impl TestFactorsRuntimeConfig {
            /// Get whether the store manager exists for the given label.
            fn has_store_manager(&self, label: &str) -> bool {
                self.key_value.as_ref().unwrap().has_store_manager(label)
            }
        }

        // Test that the default label is added if not provided.
        let toml = toml::toml! {
            [key_value_store.foo]
            type = "spin"
        };
        let runtime_config = resolve_toml(toml, "config.toml").unwrap().runtime_config;
        assert!(["default", "foo"]
            .iter()
            .all(|label| runtime_config.has_store_manager(label)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn custom_spin_key_value_works_with_custom_paths() -> anyhow::Result<()> {
        use spin_world::v2::key_value::HostStore;
        define_test_factor!(key_value: KeyValueFactor);
        let tmp_dir = tempfile::TempDir::with_prefix("example")?;
        let absolute_path = tmp_dir.path().join("foo/custom.db");
        let relative_path = tmp_dir.path().join("custom.db");
        // Check that the dbs do not exist yet - they will exist by the end of the test
        assert!(!absolute_path.exists());
        assert!(!relative_path.exists());

        let path_str = absolute_path.to_str().unwrap();
        let runtime_config = toml::toml! {
            [key_value_store.absolute]
            type = "spin"
            path = path_str

            [key_value_store.relative]
            type = "spin"
            path = "custom.db"
        };
        let factors = TestFactors {
            key_value: KeyValueFactor::new(),
        };
        let env = TestEnvironment::new(factors)
            .extend_manifest(toml::toml! {
                [component.test-component]
                source = "does-not-exist.wasm"
                key_value_stores = ["absolute", "relative"]
            })
            .runtime_config(
                resolve_toml(runtime_config, tmp_dir.path().join("runtime-config.toml"))
                    .unwrap()
                    .runtime_config,
            )?;
        let mut state = env.build_instance_state().await?;

        // Actually get a key since store creation is lazy
        let store = state.key_value.open("absolute".to_owned()).await??;
        let _ = state.key_value.get(store, "foo".to_owned()).await??;

        let store = state.key_value.open("relative".to_owned()).await??;
        let _ = state.key_value.get(store, "foo".to_owned()).await??;

        // Check that the dbs have been created
        assert!(absolute_path.exists());
        assert!(relative_path.exists());
        Ok(())
    }

    fn toml_resolver(toml: &toml::Table) -> TomlResolver<'_> {
        TomlResolver::new(
            toml,
            None,
            UserProvidedPath::Default,
            UserProvidedPath::Default,
        )
    }

    #[test]
    fn dirs_are_resolved() {
        define_test_factor!(sqlite: SqliteFactor);

        let toml = toml::toml! {
            state_dir = "/foo"
            log_dir = "/bar"
        };
        resolve_toml(toml, "config.toml").unwrap();
    }

    #[test]
    fn fails_to_resolve_with_unused_key() {
        define_test_factor!(sqlite: SqliteFactor);

        let toml = toml::toml! {
            baz = "/baz"
        };
        // assert returns an error with value "unused runtime config key(s): local_app_dir"
        let Err(e) = resolve_toml(toml, "config.toml") else {
            panic!("Should not be able to resolve unknown key");
        };
        assert_eq!(e.to_string(), "unused runtime config key(s): baz");
    }
}
