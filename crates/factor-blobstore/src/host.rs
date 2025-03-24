use anyhow::{Context, Result};
use spin_core::{async_trait, wasmtime::component::Resource};
use spin_resource_table::Table;
use spin_world::wasi::blobstore;
use tokio::sync::mpsc;
use tokio::io::{ReadHalf, SimplexStream, WriteHalf};
use tokio::sync::RwLock;
use std::{collections::HashSet, sync::Arc};
use wasmtime_wasi::WasiView;
use spin_core::wasmtime::component::ResourceTable;

use blobstore::blobstore::{self as wb};
use blobstore::container::{self as wbc};
use blobstore::types::{self as wbt};

pub use wbt::Error;

#[async_trait]
pub trait ContainerManager: Sync + Send {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error>;
    fn is_defined(&self, container_name: &str) -> bool;

    /// A human-readable summary of the given container's configuration
    ///
    /// Example: "Azure blob container 'biscuit-tin'"
    fn summary(&self, store_name: &str) -> Option<String> {
        let _ = store_name;
        None
    }
}

#[async_trait]
pub trait Container: Sync + Send {
    async fn exists(&self) -> anyhow::Result<bool>;
    async fn name(&self) -> String;
    async fn info(&self) -> anyhow::Result<wbt::ContainerMetadata>;
    async fn clear(&self) -> anyhow::Result<()>;
    async fn delete_object(&self, name: &str) -> anyhow::Result<()>;
    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()>;
    async fn has_object(&self, name: &str) -> anyhow::Result<bool>;
    async fn object_info(&self, name: &str) -> anyhow::Result<wbt::ObjectMetadata>;
    async fn get_data(&self, name: &str, start: u64, end: u64) -> anyhow::Result<Box<dyn IncomingData>>;
    async fn connect_stm(&self, name: &str, stm: ReadHalf<SimplexStream>, finished_tx: mpsc::Sender<anyhow::Result<()>>) -> anyhow::Result<()>;
    async fn list_objects(&self) -> anyhow::Result<Box<dyn ObjectNames>>;
}

#[async_trait]
pub trait ObjectNames : Send + Sync {
    async fn read(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)>;
    async fn skip(&mut self, num: u64) -> anyhow::Result<(u64,bool)>;
}

#[async_trait]
pub trait IncomingData : Send + Sync {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>>;
    fn consume_async(&mut self) -> wasmtime_wasi::pipe::AsyncReadStream;
    async fn size(&mut self) -> anyhow::Result<u64>;
}

pub struct OutgoingValue {
    read: Option<ReadHalf<SimplexStream>>,
    write: Option<WriteHalf<SimplexStream>>,
    stop_tx: Option<mpsc::Sender<()>>,
    finished_rx: Option<mpsc::Receiver<anyhow::Result<()>>>,
}

const OUTGOING_VALUE_BUF_SIZE: usize = 16 * 1024;

impl OutgoingValue {
    fn new() -> Self {
        let (read, write) = tokio::io::simplex(OUTGOING_VALUE_BUF_SIZE);
        Self {
            read: Some(read),
            write: Some(write),
            stop_tx: None,
            finished_rx: None,
       }
    }

    fn write_stream(&mut self) -> anyhow::Result<crate::AsyncWriteStream> {
        let Some(write) = self.write.take() else {
            anyhow::bail!("OutgoingValue has already returned its write stream");
        };

        let (stop_tx, stop_rx) = mpsc::channel(1);

        self.stop_tx = Some(stop_tx);

        let stm = crate::AsyncWriteStream::new_closeable(OUTGOING_VALUE_BUF_SIZE, write, stop_rx);
        Ok(stm)
    }

    fn syncers(&mut self) -> (Option<&mpsc::Sender<()>>, Option<&mut mpsc::Receiver<anyhow::Result<()>>>) {
        (self.stop_tx.as_ref(), self.finished_rx.as_mut())
    }

    fn take_read_stream(&mut self) -> anyhow::Result<(ReadHalf<SimplexStream>, mpsc::Sender<anyhow::Result<()>>)> {
        let Some(read) = self.read.take() else {
            anyhow::bail!("OutgoingValue has already been connected to a blob");
        };

        let (finished_tx, finished_rx) = mpsc::channel(1);
        self.finished_rx = Some(finished_rx);

        Ok((read, finished_tx))
    }
}

#[async_trait]
pub trait Finishable : Send + Sync {
    async fn finish(&mut self);
}

pub struct BlobStoreDispatch<'a> {
    allowed_containers: HashSet<String>,
    manager: Arc<dyn ContainerManager>,
    wasi: wasmtime_wasi::WasiImpl<WasiImplInner<'a>>,
    containers: Arc<RwLock<Table<Arc<dyn Container>>>>,
    incoming_values: Arc<RwLock<Table<Box<dyn IncomingData>>>>,
    outgoing_values: Arc<RwLock<Table<OutgoingValue>>>,
    object_names: Arc<RwLock<Table<Box<dyn ObjectNames>>>>,
}

pub struct WasiImplInner<'a> {
    pub ctx: &'a mut wasmtime_wasi::WasiCtx,
    pub table: &'a mut ResourceTable,
}

impl wasmtime_wasi::WasiView for WasiImplInner<'_> {
    fn ctx(&mut self) -> &mut wasmtime_wasi::WasiCtx {
        self.ctx
    }

    fn table(&mut self) -> &mut ResourceTable {
        self.table
    }
}

impl<'a> BlobStoreDispatch<'a> {
    pub(crate) fn new(allowed_containers: HashSet<String>,
        manager: Arc<dyn ContainerManager>,
        wasi: wasmtime_wasi::WasiImpl<WasiImplInner<'a>>,
        containers: Arc<RwLock<Table<Arc<dyn Container>>>>,
        incoming_values: Arc<RwLock<Table<Box<dyn IncomingData>>>>,
        outgoing_values: Arc<RwLock<Table<OutgoingValue>>>,
        object_names: Arc<RwLock<Table<Box<dyn ObjectNames>>>>,
    ) -> Self {
        Self {
            allowed_containers,
            manager,
            wasi,
            containers,
            incoming_values,
            outgoing_values,
            object_names,
        }
    }

    pub async fn get_container(&self, container: Resource<wb::Container>) -> anyhow::Result<Arc<dyn Container>> {
        self.containers.read().await.get(container.rep()).context("invalid container").cloned()
    }

    pub fn allowed_containers(&self) -> &HashSet<String> {
        &self.allowed_containers
    }

    async fn take_incoming_value(&mut self, resource: Resource<wbc::IncomingValue>) -> Result<Box<dyn IncomingData>, String> {
        self.incoming_values.write().await.remove(resource.rep()).ok_or_else(||
            "invalid incoming-value resource".to_string()
        )
    }
}

impl wb::Host for BlobStoreDispatch<'_> {
    async fn create_container(&mut self, _name: String) -> Result<Resource<wbc::Container>, String> {
        Err("This version of Spin does not support creating containers".to_owned())
    }

    async fn get_container(&mut self, name: String) -> Result<Resource<wbc::Container>, String> {
        if self.allowed_containers.contains(&name) {
            let container = self.manager.get(&name).await?;
            let rep = self.containers.write().await.push(container).unwrap();
            Ok(Resource::new_own(rep))
        } else {
            Err("forbidden container".to_owned())
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

    async fn copy_object(&mut self, _src: wb::ObjectId, _dest: wb::ObjectId) -> Result<(), String> {
        Err("This version of Spin does not support copying objects".to_owned())
    }

    async fn move_object(&mut self, _src: wb::ObjectId, _dest: wb::ObjectId) -> Result<(), String> {
        Err("This version of Spin does not support moving objects".to_owned())
    }
}

impl wbt::Host for BlobStoreDispatch<'_> {
    fn convert_error(&mut self, error: String) -> anyhow::Result<String> {
        Ok(error)
    }
}

impl wbt::HostIncomingValue for BlobStoreDispatch<'_> {
    async fn incoming_value_consume_sync(&mut self, self_: Resource<wbt::IncomingValue>) -> Result<Vec<u8>, String> {
        let mut incoming = self.take_incoming_value(self_).await?;
        incoming.as_mut().consume_sync().await.map_err(|e| e.to_string())
    }

    async fn incoming_value_consume_async(&mut self, self_: Resource<wbt::IncomingValue>) -> Result<Resource<wasmtime_wasi::InputStream>, String> {
        let mut incoming = self.take_incoming_value(self_).await?;
        let async_body = incoming.as_mut().consume_async();
        let host_stm: Box<dyn wasmtime_wasi::HostInputStream> = Box::new(async_body);
        let resource = self.wasi.table().push(host_stm).unwrap();
        Ok(resource)
    }

    async fn size(&mut self, self_: Resource<wbt::IncomingValue>) -> anyhow::Result<u64> {
        let mut lock = self.incoming_values.write().await;
        let incoming = lock.get_mut(self_.rep()).ok_or_else(|| anyhow::anyhow!("invalid incoming-value resource"))?;
        incoming.size().await
    }

    async fn drop(&mut self, rep: Resource<wbt::IncomingValue>) -> anyhow::Result<()> {
        self.incoming_values.write().await.remove(rep.rep());
        Ok(())
    }
}

impl wbt::HostOutgoingValue for BlobStoreDispatch<'_> {
    async fn new_outgoing_value(&mut self) -> anyhow::Result<Resource<wbt::OutgoingValue>> {
        let outgoing_value = OutgoingValue::new();
        let rep = self.outgoing_values.write().await.push(outgoing_value).unwrap();
        Ok(Resource::new_own(rep))
    }

    async fn outgoing_value_write_body(&mut self, self_: Resource<wbt::OutgoingValue>) -> anyhow::Result<Result<Resource<wasmtime_wasi::OutputStream>, ()>> {
        let mut lock = self.outgoing_values.write().await;
        let outgoing = lock.get_mut(self_.rep()).ok_or_else(||
            anyhow::anyhow!("invalid outgoing-value resource")
        )?;
        let stm = outgoing.write_stream()?;

        let host_stm: Box<dyn wasmtime_wasi::HostOutputStream> = Box::new(stm);
        let resource = self.wasi.table().push(host_stm).unwrap();

        Ok(Ok(resource))
    }

    async fn finish(&mut self, self_: Resource<wbt::OutgoingValue>) -> Result<(), String> {
        let mut lock = self.outgoing_values.write().await;
        let outgoing = lock.get_mut(self_.rep()).ok_or_else(||
            "invalid outgoing-value resource".to_string()
        )?;
        // Separate methods cause "mutable borrow while immutably borrowed" so get it all in one go
        let (stop_tx, finished_rx) = outgoing.syncers();
        let stop_tx = stop_tx.expect("shoulda had a stop_tx");
        let finished_rx = finished_rx.expect("shoulda had a finished_rx");

        stop_tx.send(()).await.expect("shoulda sent a stop");
        let result = finished_rx.recv().await;

        match result {
            None | Some(Ok(())) => Ok(()),
            Some(Err(e)) => Err(format!("{e}")),
        }
    }

    async fn drop(&mut self, rep: Resource<wbt::OutgoingValue>) -> anyhow::Result<()> {
        self.outgoing_values.write().await.remove(rep.rep());
        Ok(())
    }
}

// TODO: TBD if these belong on BSD or some other struct (like the one that maps to a Container resource JUST SAYIN)
impl wbc::Host for BlobStoreDispatch<'_> {}

impl wbc::HostContainer for BlobStoreDispatch<'_> {
    async fn name(&mut self, self_: Resource<wbc::Container>) -> Result<String, String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        Ok(container.name().await)
    }

    async fn info(&mut self, self_: Resource<wbc::Container>) -> Result<wbc::ContainerMetadata, String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        container.info().await.map_err(|e| e.to_string())
    }

    async fn get_data(&mut self, self_: Resource<wbc::Container>, name: wbc::ObjectName, start: u64, end: u64) -> Result<Resource<wbt::IncomingValue>, String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        let incoming = container.get_data(&name, start, end).await.map_err(|e| e.to_string())?;
        let rep = self.incoming_values.write().await.push(incoming).unwrap();
        Ok(Resource::new_own(rep))
    }

    async fn write_data(&mut self, self_: Resource<wbc::Container>, name: wbc::ObjectName, data: Resource<wbt::OutgoingValue>) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        let mut lock2 = self.outgoing_values.write().await;
        let outgoing = lock2.get_mut(data.rep()).ok_or_else(||
            "invalid outgoing-value resource".to_string()
        )?;

        let (stm, finished_tx) = outgoing.take_read_stream().map_err(|e| e.to_string())?;
        container.connect_stm(&name, stm, finished_tx).await.map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn list_objects(&mut self, self_: Resource<wbc::Container>) -> Result<Resource<wbc::StreamObjectNames>, String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        let names = container.list_objects().await.map_err(|e| e.to_string())?;
        let rep = self.object_names.write().await.push(names).unwrap();
        Ok(Resource::new_own(rep))
    }

    async fn delete_object(&mut self, self_: Resource<wbc::Container>, name: String) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        container.delete_object(&name).await.map_err(|e| e.to_string())
    }

    async fn delete_objects(&mut self, self_: Resource<wbc::Container>, names: Vec<String>) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        container.delete_objects(&names).await.map_err(|e| e.to_string())
    }

    async fn has_object(&mut self, self_: Resource<wbc::Container>, name: String) -> Result<bool, String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        container.has_object(&name).await.map_err(|e| e.to_string())
    }

    async fn object_info(&mut self, self_: Resource<wbc::Container>, name: String) -> Result<wbt::ObjectMetadata, String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        container.object_info(&name).await.map_err(|e| e.to_string())
    }

    async fn clear(&mut self, self_: Resource<wbc::Container>) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock.get(self_.rep()).ok_or_else(||
            "invalid container resource".to_string()
        )?;
        container.clear().await.map_err(|e| e.to_string())
    }

    async fn drop(&mut self, rep: Resource<wbc::Container>) -> anyhow::Result<()> {
        self.containers.write().await.remove(rep.rep());
        Ok(())
    }
}

impl wbc::HostStreamObjectNames for BlobStoreDispatch<'_> {
    async fn read_stream_object_names(&mut self, self_: Resource<wbc::StreamObjectNames>, len: u64) -> Result<(Vec<String>,bool), String> {
        let mut lock = self.object_names.write().await;
        let object_names = lock.get_mut(self_.rep()).ok_or_else(||
            "invalid stream-object-names resource".to_string()
        )?;
        object_names.read(len).await.map_err(|e| e.to_string())
    }

    async fn skip_stream_object_names(&mut self, self_: Resource<wbc::StreamObjectNames>, num: u64) -> Result<(u64,bool), String> {
        let mut lock = self.object_names.write().await;
        let object_names = lock.get_mut(self_.rep()).ok_or_else(||
            "invalid stream-object-names resource".to_string()
        )?;
        object_names.skip(num).await.map_err(|e| e.to_string())
    }

    async fn drop(&mut self, rep: Resource<wbc::StreamObjectNames>) -> anyhow::Result<()> {
        self.object_names.write().await.remove(rep.rep());
        Ok(())
    }
}

pub fn log_error(err: impl std::fmt::Debug) -> String {
    tracing::warn!("blobstore error: {err:?}");
    format!("{err:?}")
}
