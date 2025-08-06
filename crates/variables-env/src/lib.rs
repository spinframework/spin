use std::{
    collections::HashMap,
    env::VarError,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::Deserialize;
use spin_expressions::{Key, Provider};
use spin_factors::anyhow::{self, Context as _};
use spin_world::async_trait;
use tracing::{instrument, Level};

/// Configuration for the environment variables provider.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvVariablesConfig {
    /// A prefix to add to variable names when resolving from the environment.
    ///
    /// Unless empty, joined to the variable name with an underscore.
    #[serde(default)]
    pub prefix: Option<String>,
    /// Optional path to a 'dotenv' file which will be merged into the environment.
    #[serde(default)]
    pub dotenv_path: Option<PathBuf>,
}

const DEFAULT_ENV_PREFIX: &str = "SPIN_VARIABLE";

type EnvFetcherFn = Box<dyn Fn(&str) -> Result<String, VarError> + Send + Sync>;

/// A [`Provider`] that uses environment variables.
pub struct EnvVariablesProvider {
    prefix: Option<String>,
    env_fetcher: EnvFetcherFn,
    dotenv_path: Option<PathBuf>,
    dotenv_cache: OnceLock<HashMap<String, String>>,
}

impl Default for EnvVariablesProvider {
    fn default() -> Self {
        Self {
            prefix: None,
            env_fetcher: Box::new(|s| std::env::var(s)),
            dotenv_path: Some(".env".into()),
            dotenv_cache: Default::default(),
        }
    }
}

impl EnvVariablesProvider {
    /// Creates a new EnvProvider.
    ///
    /// * `prefix` - The string prefix to use to distinguish an environment variable that should be used.
    ///   If not set, the default prefix is used.
    /// * `env_fetcher` - The function to use to fetch an environment variable.
    /// * `dotenv_path` - The path to the .env file to load environment variables from. If not set,
    ///   no .env file is loaded.
    pub fn new(
        prefix: Option<impl Into<String>>,
        env_fetcher: impl Fn(&str) -> Result<String, VarError> + Send + Sync + 'static,
        dotenv_path: Option<PathBuf>,
    ) -> Self {
        Self {
            prefix: prefix.map(Into::into),
            dotenv_path,
            env_fetcher: Box::new(env_fetcher),
            dotenv_cache: Default::default(),
        }
    }

    /// Gets the value of a variable from the environment.
    fn get_sync(&self, key: &Key) -> anyhow::Result<Option<String>> {
        let prefix = self
            .prefix
            .clone()
            .unwrap_or_else(|| DEFAULT_ENV_PREFIX.to_string());

        let upper_key = key.as_ref().to_ascii_uppercase();
        let env_key = format!("{prefix}_{upper_key}");

        self.query_env(&env_key)
    }

    /// Queries the environment for a variable defaulting to dotenv.
    fn query_env(&self, env_key: &str) -> anyhow::Result<Option<String>> {
        match (self.env_fetcher)(env_key) {
            Err(std::env::VarError::NotPresent) => self.get_dotenv(env_key),
            other => other
                .map(Some)
                .with_context(|| format!("failed to resolve env var {env_key}")),
        }
    }

    fn get_dotenv(&self, key: &str) -> anyhow::Result<Option<String>> {
        let Some(dotenv_path) = self.dotenv_path.as_deref() else {
            return Ok(None);
        };
        let cache = match self.dotenv_cache.get() {
            Some(cache) => cache,
            None => {
                let cache = load_dotenv(dotenv_path)?;
                let _ = self.dotenv_cache.set(cache);
                // Safe to unwrap because we just set the cache.
                // Ensures we always get the first value set.
                self.dotenv_cache.get().unwrap()
            }
        };
        Ok(cache.get(key).cloned())
    }
}

impl std::fmt::Debug for EnvVariablesProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvProvider")
            .field("prefix", &self.prefix)
            .field("dotenv_path", &self.dotenv_path)
            .finish()
    }
}

fn load_dotenv(dotenv_path: &Path) -> anyhow::Result<HashMap<String, String>> {
    Ok(dotenvy::from_path_iter(dotenv_path)
        .into_iter()
        .flatten()
        .collect::<Result<HashMap<String, String>, _>>()?)
}

#[async_trait]
impl Provider for EnvVariablesProvider {
    #[instrument(name = "spin_variables.get_from_env", level = Level::DEBUG, skip(self), err(level = Level::INFO))]
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        tokio::task::block_in_place(|| self.get_sync(key))
    }
}

#[cfg(test)]
mod test {
    use std::env::temp_dir;

    use super::*;

    struct TestEnv {
        map: HashMap<String, String>,
    }

    impl TestEnv {
        fn new() -> Self {
            Self {
                map: Default::default(),
            }
        }

        fn insert(&mut self, key: &str, value: &str) {
            self.map.insert(key.to_string(), value.to_string());
        }

        fn get(&self, key: &str) -> Result<String, VarError> {
            self.map.get(key).cloned().ok_or(VarError::NotPresent)
        }
    }

    #[test]
    fn provider_get() {
        let mut env = TestEnv::new();
        env.insert("TESTING_SPIN_ENV_KEY1", "val");
        let key1 = Key::new("env_key1").unwrap();
        assert_eq!(
            EnvVariablesProvider::new(Some("TESTING_SPIN"), move |key| env.get(key), None)
                .get_sync(&key1)
                .unwrap(),
            Some("val".to_string())
        );
    }

    #[test]
    fn provider_get_dotenv() {
        let dotenv_path = temp_dir().join("spin-env-provider-test");
        std::fs::write(&dotenv_path, b"TESTING_SPIN_ENV_KEY2=dotenv_val").unwrap();

        let key = Key::new("env_key2").unwrap();
        assert_eq!(
            EnvVariablesProvider::new(
                Some("TESTING_SPIN"),
                |_| Err(VarError::NotPresent),
                Some(dotenv_path)
            )
            .get_sync(&key)
            .unwrap(),
            Some("dotenv_val".to_string())
        );
    }

    #[test]
    fn provider_get_missing() {
        let key = Key::new("definitely_not_set").unwrap();
        assert_eq!(
            EnvVariablesProvider::new(
                Some("TESTING_SPIN"),
                |_| Err(VarError::NotPresent),
                Default::default()
            )
            .get_sync(&key)
            .unwrap(),
            None
        );
    }
}
