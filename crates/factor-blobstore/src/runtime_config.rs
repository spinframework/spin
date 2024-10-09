pub mod spin;

use std::{collections::HashMap, sync::Arc};

use crate::ContainerManager;

/// Runtime configuration for all key value stores.
#[derive(Default, Clone)]
pub struct RuntimeConfig {
    /// Map of store names to store managers.
    store_managers: HashMap<String, Arc<dyn ContainerManager>>,
}

impl RuntimeConfig {
    /// Adds a store manager for the store with the given label to the runtime configuration.
    ///
    /// If a store manager already exists for the given label, it will be replaced.
    pub fn add_container_manager(&mut self, label: String, store_manager: Arc<dyn ContainerManager>) {
        self.store_managers.insert(label, store_manager);
    }

    /// Returns whether a store manager exists for the store with the given label.
    pub fn has_store_manager(&self, label: &str) -> bool {
        self.store_managers.contains_key(label)
    }

    /// Returns the store manager for the store with the given label.
    pub fn get_store_manager(&self, label: &str) -> Option<Arc<dyn ContainerManager>> {
        self.store_managers.get(label).cloned()
    }
}

impl IntoIterator for RuntimeConfig {
    type Item = (String, Arc<dyn ContainerManager>);
    type IntoIter = std::collections::hash_map::IntoIter<String, Arc<dyn ContainerManager>>;

    fn into_iter(self) -> Self::IntoIter {
        self.store_managers.into_iter()
    }
}
