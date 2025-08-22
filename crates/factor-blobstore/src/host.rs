use anyhow::Result;
use spin_core::wasmtime::component::ResourceTable;
use spin_core::{async_trait, wasmtime::component::Resource};
use spin_resource_table::Table;
use spin_world::wasi::blobstore::{self as bs};
use std::{collections::HashSet, sync::Arc};
use tokio::io::{ReadHalf, SimplexStream};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

pub use bs::types::Error;

mod container;
mod incoming_value;
mod object_names;
mod outgoing_value;

pub(crate) use outgoing_value::OutgoingValue;

use crate::DelegatingContainerManager;

// TODO: I feel like the notions of "container" and "container manager" are muddled.
// This was kinda modelled on the KV StoreManager but I am not sure it has worked.
// A "container manager" actually manages only one container, making the `get` and
// `is_defined` functions seemingly redundant. More clarity and better definition
// is needed here, although the existing code does work!
//
// Part of the trouble is, I think, that the WIT has operations for "create container"
// etc. which implies a level above "container" but whose semantics are very poorly
// defined (the implication in the WIT is that a `blobstore` implementation backs
// onto exactly one provider, and if you need to deal with multiple providers then
// you need to do some double-import trickery, which does not seem right). Clarification
// sought via https://github.com/WebAssembly/wasi-blobstore/issues/27, so we may need
// to do some rework once the authors define it more fully.

/// Allows obtaining a container. The only interesting implementation is
/// [DelegatingContainerManager] (which is what [BlobStoreDispatch] uses);
/// other implementations currently manage only one container. (See comments.)
#[async_trait]
pub trait ContainerManager: Sync + Send {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error>;
    fn is_defined(&self, container_name: &str) -> bool;
}

/// A container. This represents the system or network resource defined by
/// a label mapping in the runtime config, e.g. a file system directory,
/// Azure blob storage account, or S3 bucket. This trait is implemented
/// by providers; it is the interface through which the [BlobStoreDispatch]
/// WASI host talks to the different implementations.
#[async_trait]
pub trait Container: Sync + Send {
    async fn exists(&self) -> anyhow::Result<bool>;
    async fn name(&self) -> String;
    async fn info(&self) -> anyhow::Result<bs::types::ContainerMetadata>;
    async fn clear(&self) -> anyhow::Result<()>;
    async fn delete_object(&self, name: &str) -> anyhow::Result<()>;
    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()>;
    async fn has_object(&self, name: &str) -> anyhow::Result<bool>;
    async fn object_info(&self, name: &str) -> anyhow::Result<bs::types::ObjectMetadata>;
    async fn get_data(
        &self,
        name: &str,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Box<dyn IncomingData>>;
    async fn write_data(
        &self,
        name: &str,
        data: ReadHalf<SimplexStream>,
        finished_tx: mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<()>;
    async fn list_objects(&self) -> anyhow::Result<Box<dyn ObjectNames>>;
}

/// An interface implemented by providers when listing objects.
#[async_trait]
pub trait ObjectNames: Send + Sync {
    async fn read(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)>;
    async fn skip(&mut self, num: u64) -> anyhow::Result<(u64, bool)>;
}

/// The content of a blob being read from a container. Called by the host to
/// handle WIT incoming-value methods, and implemented by providers.
/// providers
#[async_trait]
pub trait IncomingData: Send + Sync {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>>;
    fn consume_async(&mut self) -> wasmtime_wasi::p2::pipe::AsyncReadStream;
    async fn size(&mut self) -> anyhow::Result<u64>;
}

/// Implements all the WIT host interfaces for wasi-blobstore.
pub struct BlobStoreDispatch<'a> {
    allowed_containers: &'a HashSet<String>,
    manager: &'a DelegatingContainerManager,
    wasi_resources: &'a mut ResourceTable,
    containers: &'a RwLock<Table<Arc<dyn Container>>>,
    incoming_values: &'a RwLock<Table<Box<dyn IncomingData>>>,
    outgoing_values: &'a RwLock<Table<OutgoingValue>>,
    object_names: &'a RwLock<Table<Box<dyn ObjectNames>>>,
}

impl<'a> BlobStoreDispatch<'a> {
    pub(crate) fn new(
        allowed_containers: &'a HashSet<String>,
        manager: &'a DelegatingContainerManager,
        wasi_resources: &'a mut ResourceTable,
        containers: &'a RwLock<Table<Arc<dyn Container>>>,
        incoming_values: &'a RwLock<Table<Box<dyn IncomingData>>>,
        outgoing_values: &'a RwLock<Table<OutgoingValue>>,
        object_names: &'a RwLock<Table<Box<dyn ObjectNames>>>,
    ) -> Self {
        Self {
            allowed_containers,
            manager,
            wasi_resources,
            containers,
            incoming_values,
            outgoing_values,
            object_names,
        }
    }

    pub fn allowed_containers(&self) -> &HashSet<String> {
        self.allowed_containers
    }

    async fn take_incoming_value(
        &mut self,
        resource: Resource<bs::container::IncomingValue>,
    ) -> Result<Box<dyn IncomingData>, String> {
        self.incoming_values
            .write()
            .await
            .remove(resource.rep())
            .ok_or_else(|| "invalid incoming-value resource".to_string())
    }
}

impl bs::blobstore::Host for BlobStoreDispatch<'_> {
    async fn create_container(
        &mut self,
        _name: String,
    ) -> Result<Resource<bs::container::Container>, String> {
        Err("This version of Spin does not support creating containers".to_owned())
    }

    async fn get_container(
        &mut self,
        name: String,
    ) -> Result<Resource<bs::container::Container>, String> {
        if self.allowed_containers.contains(&name) {
            let container = self.manager.get(&name).await?;
            let rep = self.containers.write().await.push(container).unwrap();
            Ok(Resource::new_own(rep))
        } else {
            Err(format!("Container {name:?} not defined or access denied"))
        }
    }

    async fn delete_container(&mut self, _name: String) -> Result<(), String> {
        Err("This version of Spin does not support deleting containers".to_owned())
    }

    async fn container_exists(&mut self, name: String) -> Result<bool, String> {
        if self.allowed_containers.contains(&name) {
            let container = self.manager.get(&name).await?;
            container.exists().await.map_err(|e| e.to_string())
        } else {
            Ok(false)
        }
    }

    async fn copy_object(
        &mut self,
        _src: bs::blobstore::ObjectId,
        _dest: bs::blobstore::ObjectId,
    ) -> Result<(), String> {
        Err("This version of Spin does not support copying objects".to_owned())
    }

    async fn move_object(
        &mut self,
        _src: bs::blobstore::ObjectId,
        _dest: bs::blobstore::ObjectId,
    ) -> Result<(), String> {
        Err("This version of Spin does not support moving objects".to_owned())
    }
}

impl bs::types::Host for BlobStoreDispatch<'_> {
    fn convert_error(&mut self, error: String) -> anyhow::Result<String> {
        Ok(error)
    }
}
