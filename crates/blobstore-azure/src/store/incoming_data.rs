use anyhow::Result;
use azure_core::Pageable;
use azure_storage_blobs::blob::operations::GetBlobResponse;
use azure_storage_blobs::prelude::BlobClient;
use futures::StreamExt;
use spin_core::async_trait;
use tokio::sync::Mutex;

pub struct AzureIncomingData {
    // The Mutex is used to make it Send
    stm: Mutex<Option<Pageable<GetBlobResponse, azure_core::error::Error>>>,
    client: BlobClient,
}

impl AzureIncomingData {
    pub fn new(client: BlobClient, range: azure_core::request_options::Range) -> Self {
        let stm = client.get().range(range).into_stream();
        Self {
            stm: Mutex::new(Some(stm)),
            client,
        }
    }

    fn consume_async_impl(&mut self) -> wasmtime_wasi::p2::pipe::AsyncReadStream {
        use futures::TryStreamExt;
        use tokio_util::compat::FuturesAsyncReadCompatExt;
        let stm = self.consume_as_stream();
        let ar = stm.into_async_read();
        let arr = ar.compat();
        wasmtime_wasi::p2::pipe::AsyncReadStream::new(arr)
    }

    fn consume_as_stream(
        &mut self,
    ) -> impl futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>> {
        let opt_stm = self.stm.get_mut();
        let stm = opt_stm.take().unwrap();
        stm.flat_map(|chunk| streamify_chunk(chunk.unwrap().data))
    }
}

fn streamify_chunk(
    chunk: azure_core::ResponseBody,
) -> impl futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    chunk.map(|c| Ok(c.unwrap().to_vec()))
}

#[async_trait]
impl spin_factor_blobstore::IncomingData for AzureIncomingData {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut data = vec![];
        let Some(pageable) = self.stm.get_mut() else {
            anyhow::bail!("oh no");
        };

        loop {
            let Some(chunk) = pageable.next().await else {
                break;
            };
            let chunk = chunk.unwrap();
            let by = chunk.data.collect().await.unwrap();
            data.extend(by.to_vec());
        }

        Ok(data)
    }

    fn consume_async(&mut self) -> wasmtime_wasi::p2::pipe::AsyncReadStream {
        self.consume_async_impl()
    }

    async fn size(&mut self) -> anyhow::Result<u64> {
        // TODO: in theory this should be infallible once we have the IncomingData
        // object. But in practice if we use the Pageable for that we don't get it until
        // we do the first read. So that would force us to either pre-fetch the
        // first chunk or to issue a properties request *just in case* size() was
        // called. So I'm making it fallible for now.
        Ok(self
            .client
            .get_properties()
            .await?
            .blob
            .properties
            .content_length)
    }
}
