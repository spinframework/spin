use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};

use spin_core::async_trait;
use spin_factor_blobstore::runtime_config::spin::MakeBlobStore;
use tokio::sync::RwLock;

/// A key-value store that uses Azure Cosmos as the backend.
#[derive(Default)]
pub struct MemoryBlobStore {
    _priv: (),
}

impl MemoryBlobStore {
    /// Creates a new `AzureBlobStore`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MakeBlobStore for MemoryBlobStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "in_memory";

    type RuntimeConfig = MemoryBlobStoreRuntimeConfig;

    type ContainerManager = BlobStoreInMemory;

    fn make_store(
        &self,
        _runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager> {
        Ok(BlobStoreInMemory::new())
    }
}

pub struct BlobStoreInMemory {
    containers: Arc<RwLock<HashMap<String, Arc<InMemoryContainer>>>>,
}

impl BlobStoreInMemory {
    fn new() -> Self {
        Self {
            containers: Default::default(),
        }
    }
}

/// The serialized runtime configuration for the in memory blob store.
#[derive(Deserialize, Serialize)]
pub struct MemoryBlobStoreRuntimeConfig {
    ignored: Option<String>,
}

#[async_trait]
impl spin_factor_blobstore::ContainerManager for BlobStoreInMemory {
    async fn get(&self, name: &str) -> Result<Arc<dyn spin_factor_blobstore::Container>, String> {
        let mut containers = self.containers.write().await;
        match containers.get(name) {
            Some(c) => Ok(c.clone()),
            None => {
                let container = Arc::new(InMemoryContainer::new(name));
                containers.insert(name.to_owned(), container.clone());
                Ok(container)
            }
        }
    }

    fn is_defined(&self, _container_name: &str) -> bool {
        true
    }
}

struct InMemoryContainer {
    name: String,
    blobs: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl InMemoryContainer {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            blobs: Default::default(),
        }
    }

    async fn read(&self) -> tokio::sync::RwLockReadGuard<HashMap<String, Vec<u8>>> {
        self.blobs.read().await
    }

    async fn write(&self) -> tokio::sync::RwLockWriteGuard<HashMap<String, Vec<u8>>> {
        self.blobs.write().await
    }
}

#[async_trait]
impl spin_factor_blobstore::Container for InMemoryContainer {
    async fn exists(&self) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn name(&self) -> String {
        self.name.clone()
    }
    async fn info(&self) -> anyhow::Result<spin_factor_blobstore::ContainerMetadata> {
        todo!()
    }
    async fn clear(&self) -> anyhow::Result<()> {
        self.write().await.clear();
        Ok(())
    }
    async fn delete_object(&self, name: &str) -> anyhow::Result<()> {
        self.write().await.remove(name);
        Ok(())
    }
    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()> {
        self.write().await.retain(|k, _| !names.contains(k));
        Ok(())
    }
    async fn has_object(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self.read().await.contains_key(name))
    }
    async fn object_info(&self, name: &str) -> anyhow::Result<spin_factor_blobstore::ObjectMetadata> {
        let size = self.blobs.read().await.get(name).ok_or_else(|| anyhow::anyhow!("blob not found"))?.len();
        Ok(spin_factor_blobstore::ObjectMetadata {
            name: name.to_string(),
            container: self.name.to_string(),
            created_at: 0,
            size: size.try_into().unwrap(),
        })
    }
    async fn get_data(&self, name: &str, start: u64, end: u64) -> anyhow::Result<Box<dyn spin_factor_blobstore::IncomingData>> {
        let data = self.read().await.get(name).ok_or_else(|| anyhow::anyhow!("blob not found"))?.clone();

        let start = start.try_into().unwrap();
        let end = end.try_into().unwrap();

        let data = if end >= data.len() {
            data[start..].to_vec()
        } else {
            data[start..=end].to_vec()
        };

        Ok(Box::new(InMemoryBlobContent { data }))
    }
    async fn list_objects(&self) -> anyhow::Result<Box<dyn spin_factor_blobstore::ObjectNames>> {
        let blobs = self.read().await;
        let names = blobs.keys().map(|k| k.to_string()).collect();
        Ok(Box::new(InMemoryBlobNames { names }))
    }
}

struct InMemoryBlobContent {
    data: Vec<u8>,
}

#[async_trait]
impl spin_factor_blobstore::IncomingData for InMemoryBlobContent {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>> {
        Ok(self.data.clone())
    }

    fn consume_async(&mut self) -> wasmtime_wasi::pipe::AsyncReadStream {
        use futures::TryStreamExt;
        use tokio_util::compat::FuturesAsyncReadCompatExt;
        let stm = futures::stream::iter([Ok(self.data.clone())]);
        let ar = stm.into_async_read().compat();
        wasmtime_wasi::pipe::AsyncReadStream::new(ar)
    }

    async fn size(&mut self) -> anyhow::Result<u64> {
        Ok(self.data.len().try_into()?)
    }
}

struct InMemoryBlobNames {
    names: Vec<String>,
}

#[async_trait]
impl spin_factor_blobstore::ObjectNames for InMemoryBlobNames {
    async fn read(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        let len = len.try_into().unwrap();
        if len > self.names.len() {
            Ok((self.names.drain(..).collect(), false))
        } else {
            let taken = self.names.drain(0..len).collect();
            Ok((taken, !self.names.is_empty()))
        }
    }

    async fn skip(&mut self, num: u64) -> anyhow::Result<(u64,bool)> {
        let len = num.try_into().unwrap();
        let (count, more) = if len > self.names.len() {
            (self.names.drain(..).len(), false)
        } else {
            let taken = self.names.drain(0..len).len();
            (taken, !self.names.is_empty())
        };
        Ok((count.try_into().unwrap(), more))
    }
}
