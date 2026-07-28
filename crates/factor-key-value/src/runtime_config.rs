pub mod spin;

use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::StoreManager;

/// Runtime configuration for all key value stores.
#[derive(Default, Clone)]
pub struct RuntimeConfig {
    /// Map of store names to store managers.
    store_managers: HashMap<String, Arc<dyn StoreManager>>,
    /// Maximum number of concurrent in-flight key-value operations for this app.
    max_concurrent_operations: Option<usize>,
    /// Optional timeout for waiting to acquire a key-value operation permit.
    wait_timeout: Option<Duration>,
}

impl RuntimeConfig {
    /// Adds a store manager for the store with the given label to the runtime configuration.
    ///
    /// If a store manager already exists for the given label, it will be replaced.
    pub fn add_store_manager(&mut self, label: String, store_manager: Arc<dyn StoreManager>) {
        self.store_managers.insert(label, store_manager);
    }

    /// Returns whether a store manager exists for the store with the given label.
    pub fn has_store_manager(&self, label: &str) -> bool {
        self.store_managers.contains_key(label)
    }

    /// Returns the store manager for the store with the given label.
    pub fn get_store_manager(&self, label: &str) -> Option<Arc<dyn StoreManager>> {
        self.store_managers.get(label).cloned()
    }

    /// Sets the maximum number of concurrent in-flight key-value operations for this app.
    pub fn set_max_concurrent_operations(&mut self, max: Option<usize>) {
        self.max_concurrent_operations = max;
    }

    /// Sets the timeout used when waiting to acquire a key-value operation permit.
    pub fn set_wait_timeout(&mut self, wait_timeout: Option<Duration>) {
        self.wait_timeout = wait_timeout;
    }

    /// Returns the maximum number of concurrent in-flight key-value operations for this app.
    pub fn max_concurrent_operations(&self) -> Option<usize> {
        self.max_concurrent_operations
    }

    /// Returns the timeout used when waiting to acquire a key-value operation permit.
    pub fn wait_timeout(&self) -> Option<Duration> {
        self.wait_timeout
    }
}

impl IntoIterator for RuntimeConfig {
    type Item = (String, Arc<dyn StoreManager>);
    type IntoIter = std::collections::hash_map::IntoIter<String, Arc<dyn StoreManager>>;

    fn into_iter(self) -> Self::IntoIter {
        self.store_managers.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;
    use std::time::Duration;

    #[test]
    fn runtime_config_can_set_operation_limits() {
        let mut config = RuntimeConfig::default();
        assert_eq!(config.max_concurrent_operations(), None);
        assert_eq!(config.wait_timeout(), None);

        config.set_max_concurrent_operations(Some(7));
        config.set_wait_timeout(Some(Duration::from_millis(250)));

        assert_eq!(config.max_concurrent_operations(), Some(7));
        assert_eq!(config.wait_timeout(), Some(Duration::from_millis(250)));
    }
}
