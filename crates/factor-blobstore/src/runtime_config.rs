pub mod spin;

use std::{collections::HashMap, sync::Arc};

use crate::ContainerManager;

/// Runtime configuration for all blob containers.
#[derive(Default, Clone)]
pub struct RuntimeConfig {
    /// Map of container names to container managers.
    container_managers: HashMap<String, Arc<dyn ContainerManager>>,
}

impl RuntimeConfig {
    /// Adds a container manager for the container with the given label to the runtime configuration.
    ///
    /// If a container manager already exists for the given label, it will be replaced.
    pub fn add_container_manager(
        &mut self,
        label: String,
        container_manager: Arc<dyn ContainerManager>,
    ) {
        self.container_managers.insert(label, container_manager);
    }

    /// Returns whether a container manager exists for the given label.
    pub fn has_container_manager(&self, label: &str) -> bool {
        self.container_managers.contains_key(label)
    }

    /// Returns the container manager for the container with the given label.
    pub fn get_container_manager(&self, label: &str) -> Option<Arc<dyn ContainerManager>> {
        self.container_managers.get(label).cloned()
    }
}

impl IntoIterator for RuntimeConfig {
    type Item = (String, Arc<dyn ContainerManager>);
    type IntoIter = std::collections::hash_map::IntoIter<String, Arc<dyn ContainerManager>>;

    fn into_iter(self) -> Self::IntoIter {
        self.container_managers.into_iter()
    }
}
