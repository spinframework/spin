use spin_core::wasmtime::component::Resource;
use spin_world::wasi::blobstore::container::{HostStreamObjectNames, StreamObjectNames};

use super::BlobStoreDispatch;

impl HostStreamObjectNames for BlobStoreDispatch<'_> {
    async fn read_stream_object_names(
        &mut self,
        self_: Resource<StreamObjectNames>,
        len: u64,
    ) -> Result<(Vec<String>, bool), String> {
        let mut lock = self.object_names.write().await;
        let object_names = lock
            .get_mut(self_.rep())
            .ok_or_else(|| "invalid stream-object-names resource".to_string())?;
        object_names.read(len).await.map_err(|e| e.to_string())
    }

    async fn skip_stream_object_names(
        &mut self,
        self_: Resource<StreamObjectNames>,
        num: u64,
    ) -> Result<(u64, bool), String> {
        let mut lock = self.object_names.write().await;
        let object_names = lock
            .get_mut(self_.rep())
            .ok_or_else(|| "invalid stream-object-names resource".to_string())?;
        object_names.skip(num).await.map_err(|e| e.to_string())
    }

    async fn drop(&mut self, rep: Resource<StreamObjectNames>) -> anyhow::Result<()> {
        self.object_names.write().await.remove(rep.rep());
        Ok(())
    }
}
