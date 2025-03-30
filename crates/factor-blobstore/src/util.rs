use crate::{Container, ContainerManager, Error};
use spin_core::async_trait;
use std::{collections::HashMap, sync::Arc};

/// A [`ContainerManager`] which delegates to other `ContainerManager`s based on the store label.
pub struct DelegatingContainerManager {
    delegates: HashMap<String, Arc<dyn ContainerManager>>,
}

impl DelegatingContainerManager {
    pub fn new(delegates: impl IntoIterator<Item = (String, Arc<dyn ContainerManager>)>) -> Self {
        let delegates = delegates.into_iter().collect();
        Self { delegates }
    }
}

#[async_trait]
impl ContainerManager for DelegatingContainerManager {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error> {
        match self.delegates.get(name) {
            Some(store) => store.get(name).await,
            None => Err("no such container".to_string()),
        }
    }

    fn is_defined(&self, store_name: &str) -> bool {
        self.delegates.contains_key(store_name)
    }
}
