use spin_core::wasmtime::component::Resource;
use spin_world::wasi::blobstore::types::HostOutgoingValue;
use spin_world::wasi::blobstore::{self as bs};
use tokio::io::{ReadHalf, SimplexStream, WriteHalf};
use tokio::sync::mpsc;

use super::BlobStoreDispatch;

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

    fn syncers(
        &mut self,
    ) -> (
        Option<&mpsc::Sender<()>>,
        Option<&mut mpsc::Receiver<anyhow::Result<()>>>,
    ) {
        (self.stop_tx.as_ref(), self.finished_rx.as_mut())
    }

    pub(crate) fn take_read_stream(
        &mut self,
    ) -> anyhow::Result<(ReadHalf<SimplexStream>, mpsc::Sender<anyhow::Result<()>>)> {
        let Some(read) = self.read.take() else {
            anyhow::bail!("OutgoingValue has already been connected to a blob");
        };

        let (finished_tx, finished_rx) = mpsc::channel(1);
        self.finished_rx = Some(finished_rx);

        Ok((read, finished_tx))
    }
}

impl HostOutgoingValue for BlobStoreDispatch<'_> {
    async fn new_outgoing_value(&mut self) -> anyhow::Result<Resource<bs::types::OutgoingValue>> {
        let outgoing_value = OutgoingValue::new();
        let rep = self
            .outgoing_values
            .write()
            .await
            .push(outgoing_value)
            .unwrap();
        Ok(Resource::new_own(rep))
    }

    async fn outgoing_value_write_body(
        &mut self,
        self_: Resource<bs::types::OutgoingValue>,
    ) -> anyhow::Result<Result<Resource<wasmtime_wasi::p2::bindings::io::streams::OutputStream>, ()>>
    {
        let mut lock = self.outgoing_values.write().await;
        let outgoing = lock
            .get_mut(self_.rep())
            .ok_or_else(|| anyhow::anyhow!("invalid outgoing-value resource"))?;
        let stm = outgoing.write_stream()?;

        let host_stm: Box<dyn wasmtime_wasi::p2::OutputStream> = Box::new(stm);
        let resource = self.wasi_resources.push(host_stm).unwrap();

        Ok(Ok(resource))
    }

    async fn finish(&mut self, self_: Resource<bs::types::OutgoingValue>) -> Result<(), String> {
        let mut lock = self.outgoing_values.write().await;
        let outgoing = lock
            .get_mut(self_.rep())
            .ok_or_else(|| "invalid outgoing-value resource".to_string())?;
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

    async fn drop(&mut self, rep: Resource<bs::types::OutgoingValue>) -> anyhow::Result<()> {
        self.outgoing_values.write().await.remove(rep.rep());
        Ok(())
    }
}
