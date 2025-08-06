use std::{collections::HashMap, hash::Hash, sync::Arc};

use serde::Deserialize;
use spin_expressions::{async_trait::async_trait, Key, Provider};
use spin_factors::anyhow;

pub use source::*;
mod source;

/// A [`Provider`] that reads variables from an static map.
#[derive(Debug, Deserialize, Clone)]
pub struct StaticVariablesProvider {
    values: Arc<HashMap<String, String>>,
}

#[async_trait]
impl Provider for StaticVariablesProvider {
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        Ok(self.values.get(key.as_str()).cloned())
    }
}

impl StaticVariablesProvider {
    /// Creates a new `StaticVariablesProvider` with the given key-value pairs.
    pub fn new<K, V>(values: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<String> + Eq + Hash,
        V: Into<String>,
    {
        let values = values
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        Self {
            values: Arc::new(values),
        }
    }
}
