use anyhow::Result;
use spin_core::wasmtime::component::Resource;
use spin_world::wasi::blobstore::{self as bs};

use super::BlobStoreDispatch;

impl bs::container::Host for BlobStoreDispatch<'_> {}

impl bs::container::HostContainer for BlobStoreDispatch<'_> {
    async fn name(&mut self, self_: Resource<bs::container::Container>) -> Result<String, String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        Ok(container.name().await)
    }

    async fn info(
        &mut self,
        self_: Resource<bs::container::Container>,
    ) -> Result<bs::container::ContainerMetadata, String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        container.info().await.map_err(|e| e.to_string())
    }

    async fn get_data(
        &mut self,
        self_: Resource<bs::container::Container>,
        name: bs::container::ObjectName,
        start: u64,
        end: u64,
    ) -> Result<Resource<bs::types::IncomingValue>, String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        let incoming = container
            .get_data(&name, start, end)
            .await
            .map_err(|e| e.to_string())?;
        let rep = self.incoming_values.write().await.push(incoming).unwrap();
        Ok(Resource::new_own(rep))
    }

    async fn write_data(
        &mut self,
        self_: Resource<bs::container::Container>,
        name: bs::container::ObjectName,
        data: Resource<bs::types::OutgoingValue>,
    ) -> Result<(), String> {
        let lock_c = self.containers.read().await;
        let container = lock_c
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        let mut lock_ov = self.outgoing_values.write().await;
        let outgoing = lock_ov
            .get_mut(data.rep())
            .ok_or_else(|| "invalid outgoing-value resource".to_string())?;

        let (stm, finished_tx) = outgoing.take_read_stream().map_err(|e| e.to_string())?;
        container
            .write_data(&name, stm, finished_tx)
            .await
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn list_objects(
        &mut self,
        self_: Resource<bs::container::Container>,
    ) -> Result<Resource<bs::container::StreamObjectNames>, String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        let names = container.list_objects().await.map_err(|e| e.to_string())?;
        let rep = self.object_names.write().await.push(names).unwrap();
        Ok(Resource::new_own(rep))
    }

    async fn delete_object(
        &mut self,
        self_: Resource<bs::container::Container>,
        name: String,
    ) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        container
            .delete_object(&name)
            .await
            .map_err(|e| e.to_string())
    }

    async fn delete_objects(
        &mut self,
        self_: Resource<bs::container::Container>,
        names: Vec<String>,
    ) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        container
            .delete_objects(&names)
            .await
            .map_err(|e| e.to_string())
    }

    async fn has_object(
        &mut self,
        self_: Resource<bs::container::Container>,
        name: String,
    ) -> Result<bool, String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        container.has_object(&name).await.map_err(|e| e.to_string())
    }

    async fn object_info(
        &mut self,
        self_: Resource<bs::container::Container>,
        name: String,
    ) -> Result<bs::types::ObjectMetadata, String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        container
            .object_info(&name)
            .await
            .map_err(|e| e.to_string())
    }

    async fn clear(&mut self, self_: Resource<bs::container::Container>) -> Result<(), String> {
        let lock = self.containers.read().await;
        let container = lock
            .get(self_.rep())
            .ok_or_else(|| "invalid container resource".to_string())?;
        container.clear().await.map_err(|e| e.to_string())
    }

    async fn drop(&mut self, rep: Resource<bs::container::Container>) -> anyhow::Result<()> {
        self.containers.write().await.remove(rep.rep());
        Ok(())
    }
}
