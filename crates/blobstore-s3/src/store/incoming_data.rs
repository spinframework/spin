use aws_sdk_s3::operation::get_object;

use anyhow::Result;
use spin_core::async_trait;

pub struct S3IncomingData {
    get_obj_output: Option<get_object::GetObjectOutput>,
}

impl S3IncomingData {
    pub fn new(get_obj_output: get_object::GetObjectOutput) -> Self {
        Self {
            get_obj_output: Some(get_obj_output),
        }
    }

    /// Destructively takes the GetObjectOutput from self.
    /// After this self will be unusable; but this cannot
    /// consume self for resource lifetime reasons.
    fn take_output(&mut self) -> get_object::GetObjectOutput {
        self.get_obj_output
            .take()
            .expect("GetObject response was already consumed")
    }

    fn consume_async_impl(&mut self) -> wasmtime_wasi::p2::pipe::AsyncReadStream {
        use futures::TryStreamExt;
        use tokio_util::compat::FuturesAsyncReadCompatExt;
        let stream = self.consume_as_stream();
        let reader = stream.into_async_read().compat();
        wasmtime_wasi::p2::pipe::AsyncReadStream::new(reader)
    }

    fn consume_as_stream(
        &mut self,
    ) -> impl futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>> {
        use futures::StreamExt;
        let get_obj_output = self.take_output();
        let reader = get_obj_output.body.into_async_read();
        let stream = tokio_util::io::ReaderStream::new(reader);
        stream.map(|chunk| chunk.map(|b| b.to_vec()))
    }
}

#[async_trait]
impl spin_factor_blobstore::IncomingData for S3IncomingData {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>> {
        let get_obj_output = self.take_output();
        Ok(get_obj_output.body.collect().await?.to_vec())
    }

    fn consume_async(&mut self) -> wasmtime_wasi::p2::pipe::AsyncReadStream {
        self.consume_async_impl()
    }

    async fn size(&mut self) -> anyhow::Result<u64> {
        use anyhow::Context;
        let goo = self
            .get_obj_output
            .as_ref()
            .context("object was already consumed")?;
        Ok(goo
            .content_length()
            .context("content-length not returned")?
            .try_into()?)
    }
}
