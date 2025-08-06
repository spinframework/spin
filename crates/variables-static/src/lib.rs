use std::{collections::HashMap, sync::Arc};

use serde::Deserialize;
use spin_expressions::{async_trait::async_trait, Key, Provider};
use spin_factors::anyhow;

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
