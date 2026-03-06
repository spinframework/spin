use crate::{Cas, Error, Store, StoreManager, SwapError};
use spin_core::async_trait;
use std::{collections::HashMap, sync::Arc};

/// A [`StoreManager`] which delegates to other `StoreManager`s based on the store label.
pub struct DelegatingStoreManager {
    delegates: HashMap<String, Arc<dyn StoreManager>>,
}

impl DelegatingStoreManager {
    pub fn new(delegates: impl IntoIterator<Item = (String, Arc<dyn StoreManager>)>) -> Self {
        let delegates = delegates.into_iter().collect();
        Self { delegates }
    }
}

#[async_trait]
impl StoreManager for DelegatingStoreManager {
    async fn get(&self, name: &str) -> Result<Arc<dyn Store>, Error> {
        match self.delegates.get(name) {
            Some(store) => store.get(name).await,
            None => Err(Error::NoSuchStore),
        }
    }

    fn is_defined(&self, store_name: &str) -> bool {
        self.delegates.contains_key(store_name)
    }

    fn summary(&self, store_name: &str) -> Option<String> {
        if let Some(store) = self.delegates.get(store_name) {
            return store.summary(store_name);
        }
        None
    }
}

/// The well-known label for instance-scoped stores.
pub const INSTANCE_STORE_LABEL: &str = "instance-store";

/// A [`StoreManager`] wrapper that auto-namespaces the "instance-store" label
/// with a stateful component's instance ID, so each instance sees isolated data.
pub struct InstanceScopedStoreManager {
    inner: Arc<dyn StoreManager>,
    instance_id: String,
}

impl InstanceScopedStoreManager {
    pub fn new(inner: Arc<dyn StoreManager>, instance_id: String) -> Self {
        Self { inner, instance_id }
    }
}

#[async_trait]
impl StoreManager for InstanceScopedStoreManager {
    async fn get(&self, name: &str) -> Result<Arc<dyn Store>, Error> {
        let store = self.inner.get(name).await?;
        if name == INSTANCE_STORE_LABEL {
            Ok(Arc::new(InstanceScopedStore {
                inner: store,
                prefix: format!("{}/", self.instance_id),
            }))
        } else {
            Ok(store)
        }
    }

    fn is_defined(&self, store_name: &str) -> bool {
        self.inner.is_defined(store_name)
    }

    fn summary(&self, store_name: &str) -> Option<String> {
        self.inner.summary(store_name)
    }
}

/// A [`Store`] wrapper that prefixes all keys with an instance ID,
/// providing per-instance key isolation within a shared underlying store.
struct InstanceScopedStore {
    inner: Arc<dyn Store>,
    prefix: String,
}

impl InstanceScopedStore {
    fn prefixed_key(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }
}

#[async_trait]
impl Store for InstanceScopedStore {
    async fn after_open(&self) -> Result<(), Error> {
        self.inner.after_open().await
    }

    async fn get(&self, key: &str, max_result_bytes: usize) -> Result<Option<Vec<u8>>, Error> {
        self.inner.get(&self.prefixed_key(key), max_result_bytes).await
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<(), Error> {
        self.inner.set(&self.prefixed_key(key), value).await
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        self.inner.delete(&self.prefixed_key(key)).await
    }

    async fn exists(&self, key: &str) -> Result<bool, Error> {
        self.inner.exists(&self.prefixed_key(key)).await
    }

    async fn get_keys(&self, max_result_bytes: usize) -> Result<Vec<String>, Error> {
        let keys = self.inner.get_keys(max_result_bytes).await?;
        Ok(keys
            .into_iter()
            .filter_map(|k| k.strip_prefix(&self.prefix).map(String::from))
            .collect())
    }

    async fn get_many(
        &self,
        keys: Vec<String>,
        max_result_bytes: usize,
    ) -> Result<Vec<(String, Option<Vec<u8>>)>, Error> {
        let prefixed_keys: Vec<String> = keys.iter().map(|k| self.prefixed_key(k)).collect();
        let results = self.inner.get_many(prefixed_keys, max_result_bytes).await?;
        Ok(results
            .into_iter()
            .map(|(k, v)| {
                let unprefixed = k.strip_prefix(&self.prefix).unwrap_or(&k).to_string();
                (unprefixed, v)
            })
            .collect())
    }

    async fn set_many(&self, key_values: Vec<(String, Vec<u8>)>) -> Result<(), Error> {
        let prefixed: Vec<(String, Vec<u8>)> = key_values
            .into_iter()
            .map(|(k, v)| (self.prefixed_key(&k), v))
            .collect();
        self.inner.set_many(prefixed).await
    }

    async fn delete_many(&self, keys: Vec<String>) -> Result<(), Error> {
        let prefixed: Vec<String> = keys.iter().map(|k| self.prefixed_key(k)).collect();
        self.inner.delete_many(prefixed).await
    }

    async fn increment(&self, key: String, delta: i64) -> Result<i64, Error> {
        self.inner.increment(self.prefixed_key(&key), delta).await
    }

    async fn new_compare_and_swap(
        &self,
        bucket_rep: u32,
        key: &str,
    ) -> Result<Arc<dyn Cas>, Error> {
        self.inner
            .new_compare_and_swap(bucket_rep, &self.prefixed_key(key))
            .await
    }
}
