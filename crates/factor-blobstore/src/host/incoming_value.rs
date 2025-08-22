use spin_core::wasmtime::component::Resource;
use spin_world::wasi::blobstore::{self as bs};
use wasmtime_wasi::p2::bindings::io::streams::InputStream;
use wasmtime_wasi::p2::InputStream as HostInputStream;

use super::BlobStoreDispatch;

impl bs::types::HostIncomingValue for BlobStoreDispatch<'_> {
    async fn incoming_value_consume_sync(
        &mut self,
        self_: Resource<bs::types::IncomingValue>,
    ) -> Result<Vec<u8>, String> {
        let mut incoming = self.take_incoming_value(self_).await?;
        incoming
            .as_mut()
            .consume_sync()
            .await
            .map_err(|e| e.to_string())
    }

    async fn incoming_value_consume_async(
        &mut self,
        self_: Resource<bs::types::IncomingValue>,
    ) -> Result<Resource<InputStream>, String> {
        let mut incoming = self.take_incoming_value(self_).await?;
        let async_body = incoming.as_mut().consume_async();
        let input_stream: Box<dyn HostInputStream> = Box::new(async_body);
        let resource = self.wasi_resources.push(input_stream).unwrap();
        Ok(resource)
    }

    async fn size(&mut self, self_: Resource<bs::types::IncomingValue>) -> anyhow::Result<u64> {
        let mut lock = self.incoming_values.write().await;
        let incoming = lock
            .get_mut(self_.rep())
            .ok_or_else(|| anyhow::anyhow!("invalid incoming-value resource"))?;
        incoming.size().await
    }

    async fn drop(&mut self, rep: Resource<bs::types::IncomingValue>) -> anyhow::Result<()> {
        self.incoming_values.write().await.remove(rep.rep());
        Ok(())
    }
}
