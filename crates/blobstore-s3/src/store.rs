use std::sync::Arc;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::config::http::HttpResponse as AwsHttpResponse;
use aws_sdk_s3::operation::get_object;
use aws_sdk_s3::operation::list_objects_v2;
use aws_smithy_async::future::pagination_stream::PaginationStream;
use tokio::sync::Mutex;

use anyhow::Result;
use spin_core::async_trait;
use spin_factor_blobstore::{Error, Container, ContainerManager};

pub struct BlobStoreS3 {
    builder: object_store::aws::AmazonS3Builder,
    client: async_once_cell::Lazy<
        aws_sdk_s3::Client,
        std::pin::Pin<Box<dyn std::future::Future<Output = aws_sdk_s3::Client> + Send>>,
    >,
    bucket: Option<String>,
}

/// AWS S3 runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct BlobStoreS3RuntimeConfigOptions {
    /// The access key for the AWS S3 account role.
    access_key: String,
    /// The secret key for authorization on the AWS S3 account.
    secret_key: String,
    /// The token for authorization on the AWS S3 account.
    token: Option<String>,
}

impl BlobStoreS3RuntimeConfigOptions {
    pub fn new(
        access_key: String,
        secret_key: String,
        token: Option<String>,
    ) -> Self {
        Self { access_key, secret_key, token }
    }
}

impl aws_credential_types::provider::ProvideCredentials for BlobStoreS3RuntimeConfigOptions {
    fn provide_credentials<'a>(
        &'a self,
    ) -> aws_credential_types::provider::future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        aws_credential_types::provider::future::ProvideCredentials::ready(Ok(aws_credential_types::Credentials::new(
            self.access_key.clone(),
            self.secret_key.clone(),
            self.token.clone(),
            None, // Optional expiration time
            "spin_custom_aws_provider",
        )))
    }
}

/// AWS S3 authentication options
#[derive(Clone, Debug)]
pub enum BlobStoreS3AuthOptions {
    /// Runtime Config values indicates the account and key have been specified directly
    RuntimeConfigValues(BlobStoreS3RuntimeConfigOptions),
    /// Use environment variables
    Environmental,
}

impl BlobStoreS3 {
    pub fn new(
        region: String,
        auth_options: BlobStoreS3AuthOptions,
        bucket: Option<String>,
    ) -> Result<Self> {
        let builder = match &auth_options {
            BlobStoreS3AuthOptions::RuntimeConfigValues(config) =>
                object_store::aws::AmazonS3Builder::new()
                    .with_region(&region)
                    .with_access_key_id(&config.access_key)
                    .with_secret_access_key(&config.secret_key)
                    .with_token(config.token.clone().unwrap_or_default()),
            BlobStoreS3AuthOptions::Environmental => object_store::aws::AmazonS3Builder::from_env(),
        };

        let region_clone = region.clone();
        let client_fut = Box::pin(async move {
            let sdk_config = match auth_options {
                BlobStoreS3AuthOptions::RuntimeConfigValues(config) => aws_config::SdkConfig::builder()
                    .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(config))
                    .region(aws_config::Region::new(region_clone))
                    .behavior_version(aws_config::BehaviorVersion::latest())
                    .build(),
                BlobStoreS3AuthOptions::Environmental => {
                    aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
                }
            };
            aws_sdk_s3::Client::new(&sdk_config)
        });

        Ok(Self { builder, client: async_once_cell::Lazy::from_future(client_fut), bucket })
    }
}

#[async_trait]
impl ContainerManager for BlobStoreS3 {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error> {
        let name = self.bucket.clone().unwrap_or_else(|| name.to_owned());

        let store = self.builder.clone().with_bucket_name(&name).build().map_err(|e| e.to_string())?;

        Ok(Arc::new(S3Container {
            name,
            store,
            client: self.client.get_unpin().await.clone(),
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }

    fn summary(&self, _store_name: &str) -> Option<String> {
        Some("AWS S3 blob storage".to_owned())
    }
}

struct S3Container {
    name: String,
    store: object_store::aws::AmazonS3,
    client: aws_sdk_s3::Client,
}

/// S3 doesn't provide us with a container creation time
const DUMMY_CREATED_AT: u64 = 0;

#[async_trait]
impl Container for S3Container {
    async fn exists(&self) -> anyhow::Result<bool> {
        match self.client.head_bucket().bucket(&self.name).send().await {
            Ok(_) => Ok(true),
            Err(e) => match e.as_service_error() {
                Some(se) => Ok(!se.is_not_found()),
                None => anyhow::bail!(e),
            }
        }
    }

    async fn name(&self) -> String {
        self.name.clone()
    }

    async fn info(&self) -> anyhow::Result<spin_factor_blobstore::ContainerMetadata> {
        Ok(spin_factor_blobstore::ContainerMetadata {
            name: self.name.clone(),
            created_at: DUMMY_CREATED_AT,
        })
    }

    async fn clear(&self) -> anyhow::Result<()> {
        anyhow::bail!("AWS S3 blob storage does not support clearing containers")
    }

    async fn delete_object(&self, name: &str) -> anyhow::Result<()> {
        self.client.delete_object().bucket(&self.name).key(name).send().await?;
        Ok(())
    }

    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()> {
        // TODO: are atomic semantics required? or efficiency guarantees?
        let futures = names.iter().map(|name| self.delete_object(name));
        futures::future::try_join_all(futures).await?;
        Ok(())
    }

    async fn has_object(&self, name: &str) -> anyhow::Result<bool> {
        match self.client.head_object().bucket(&self.name).key(name).send().await {
            Ok(_) => Ok(true),
            Err(e) => match e.as_service_error() {
                Some(se) => Ok(!se.is_not_found()),
                None => anyhow::bail!(e),
            }
        }
    }

    async fn object_info(&self, name: &str) -> anyhow::Result<spin_factor_blobstore::ObjectMetadata> {
        let response = self.client.head_object().bucket(&self.name).key(name).send().await?;
        Ok(spin_factor_blobstore::ObjectMetadata {
            name: name.to_string(),
            container: self.name.clone(),
            created_at: response.last_modified().and_then(|t| t.secs().try_into().ok()).unwrap_or(DUMMY_CREATED_AT),
            size: response.content_length.and_then(|l| l.try_into().ok()).unwrap_or_default(),
        })
    }

    async fn get_data(&self, name: &str, start: u64, end: u64) -> anyhow::Result<Box<dyn spin_factor_blobstore::IncomingData>> {
        let range = if end == u64::MAX {
            format!("bytes={start}-")
        } else {
            format!("bytes={start}-{end}")
        };
        let resp = self.client.get_object().bucket(&self.name).key(name).range(range).send().await?;
        Ok(Box::new(S3IncomingData::new(resp)))
    }

    async fn connect_stm(&self, name: &str, stm: tokio::io::ReadHalf<tokio::io::SimplexStream>, finished_tx: tokio::sync::mpsc::Sender<anyhow::Result<()>>) -> anyhow::Result<()> {
        let store = self.store.clone();
        let path = object_store::path::Path::from(name);

        tokio::spawn(async move {
            let conn_result = Self::connect_stm_core(stm, store, path).await;
            finished_tx.send(conn_result).await.expect("should sent finish tx");
        });

        Ok(())
    }

    async fn list_objects(&self) -> anyhow::Result<Box<dyn spin_factor_blobstore::ObjectNames>> {
        let stm = self.client.list_objects_v2().bucket(&self.name).into_paginator().send();
        Ok(Box::new(S3BlobsList::new(stm)))
    }
}

impl S3Container {
    async fn connect_stm_core(mut stm: tokio::io::ReadHalf<tokio::io::SimplexStream>, store: object_store::aws::AmazonS3, path: object_store::path::Path) -> anyhow::Result<()> {
        use object_store::ObjectStore;

        let mupload = store.put_multipart(&path).await?;
        let mut writer = object_store::WriteMultipart::new(mupload);
        loop {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0; 5 * 1024 * 1024];
            let read_amount = stm.read(&mut buf).await?;
            if read_amount == 0 {
                break;
            }
            buf.truncate(read_amount);
            writer.put(buf.into());
        }
        writer.finish().await?;

        Ok(())
    }
}

struct S3IncomingData {
    get_obj_resp: Option<get_object::GetObjectOutput>,
}

impl S3IncomingData {
    fn new(get_obj_resp: get_object::GetObjectOutput) -> Self {
        Self {
            get_obj_resp: Some(get_obj_resp),
        }
    }

    fn consume_async_impl(&mut self) -> wasmtime_wasi::pipe::AsyncReadStream {
        use futures::TryStreamExt;
        use tokio_util::compat::FuturesAsyncReadCompatExt;
        let stm = self.consume_as_stream();
        let ar = stm.into_async_read();
        let arr = ar.compat();
        wasmtime_wasi::pipe::AsyncReadStream::new(arr)
    }

    fn consume_as_stream(&mut self) -> impl futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>> {
        use futures::StreamExt;
        let rr = self.get_obj_resp.take().expect("get object resp was already consumed");
        let ar = rr.body.into_async_read();
        let s = tokio_util::io::ReaderStream::new(ar);
        s.map(|by| by.map(|b| b.to_vec()))
    }
}

struct S3BlobsList {
    stm: Mutex<PaginationStream<Result<list_objects_v2::ListObjectsV2Output, SdkError<list_objects_v2::ListObjectsV2Error, AwsHttpResponse>>>>,
    read_but_not_yet_returned: Vec<String>,
    end_stm_after_read_but_not_yet_returned: bool,
}

impl S3BlobsList {
    fn new(stm: PaginationStream<Result<list_objects_v2::ListObjectsV2Output, SdkError<list_objects_v2::ListObjectsV2Error, AwsHttpResponse>>>) -> Self {
        Self {
            stm: Mutex::new(stm),
            read_but_not_yet_returned: Default::default(),
            end_stm_after_read_but_not_yet_returned: false,
        }
    }

    async fn read_impl(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        let len: usize = len.try_into().unwrap();

        // If we have names outstanding, send that first.  (We are allowed to send less than len,
        // and so sending all pending stuff before paging, rather than trying to manage a mix of
        // pending stuff with newly retrieved chunks, simplifies the code.)
        if !self.read_but_not_yet_returned.is_empty() {
            if self.read_but_not_yet_returned.len() <= len {
                // We are allowed to send all pending names
                let to_return = self.read_but_not_yet_returned.drain(..).collect();
                return Ok((to_return, self.end_stm_after_read_but_not_yet_returned));
            } else {
                // Send as much as we can. The rest remains in the pending buffer to send,
                // so this does not represent end of stream.
                let to_return = self.read_but_not_yet_returned.drain(0..len).collect();
                return Ok((to_return, false));
            }
        }

        // Get one chunk and send as much as we can of it. Aagin, we don't need to try to
        // pack the full length here - we can send chunk by chunk.

        let Some(chunk) = self.stm.get_mut().next().await else {
            return Ok((vec![], false));
        };
        let chunk = chunk.unwrap();

        let at_end = chunk.continuation_token().is_none();
        let mut names: Vec<_> = chunk.contents.unwrap_or_default().into_iter().flat_map(|blob| blob.key).collect();

        if names.len() <= len {
            // We can send them all!
            Ok((names, at_end))
        } else {
            // We have more names than we can send in this response. Send what we can and
            // stash the rest.
            let to_return: Vec<_> = names.drain(0..len).collect();
            self.read_but_not_yet_returned = names;
            self.end_stm_after_read_but_not_yet_returned = at_end;
            Ok((to_return, false))
        }
    }
}

#[async_trait]
impl spin_factor_blobstore::IncomingData for S3IncomingData {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>> {
        let Some(goo) = self.get_obj_resp.take() else {
            anyhow::bail!("oh no");
        };

        Ok(goo.body.collect().await?.to_vec())
    }

    fn consume_async(&mut self) -> wasmtime_wasi::pipe::AsyncReadStream {
        self.consume_async_impl()
    }

    async fn size(&mut self) -> anyhow::Result<u64> {
        use anyhow::Context;
        let goo = self.get_obj_resp.as_ref().context("resp has been taken")?;
        Ok(goo.content_length().context("content-length not returned")?.try_into()?)
    }
}

#[async_trait]
impl spin_factor_blobstore::ObjectNames for S3BlobsList {
    async fn read(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        self.read_impl(len).await  // Separate function because rust-analyser gives better intellisense when async_trait isn't in the picture!
    }

    async fn skip(&mut self, num: u64) -> anyhow::Result<(u64, bool)> {
        // TODO: there is a question (raised as an issue on the repo) about the required behaviour
        // here. For now I assume that skipping fewer than `num` is allowed as long as we are
        // honest about it. Because it is easier that is why.
        let (skipped, at_end) = self.read_impl(num).await?;
        Ok((skipped.len().try_into().unwrap(), at_end))
    }
}
