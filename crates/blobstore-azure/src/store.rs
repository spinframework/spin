use std::sync::Arc;
use tokio::sync::Mutex;

use anyhow::Result;
// use azure_data_cosmos::{
//     prelude::{AuthorizationToken, CollectionClient, CosmosClient, Query},
//     CosmosEntity,
// };
use azure_storage_blobs::prelude::{BlobServiceClient, ContainerClient};
// use futures::StreamExt;
// use serde::{Deserialize, Serialize};
use spin_core::async_trait;
use spin_factor_blobstore::{Error, Container, ContainerManager};

pub struct BlobStoreAzureBlob {
    client: BlobServiceClient,
    // client: CollectionClient,
}

/// Azure Cosmos Key / Value runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct BlobStoreAzureRuntimeConfigOptions {
    account: String,
    key: String,
}

impl BlobStoreAzureRuntimeConfigOptions {
    pub fn new(account: String, key: String) -> Self {
        Self { account, key }
    }
}

/// Azure Cosmos Key / Value enumeration for the possible authentication options
#[derive(Clone, Debug)]
pub enum BlobStoreAzureAuthOptions {
    /// Runtime Config values indicates the account and key have been specified directly
    RuntimeConfigValues(BlobStoreAzureRuntimeConfigOptions),
    /// Environmental indicates that the environment variables of the process should be used to
    /// create the StorageCredentials for the storage client. For now this uses old school credentials:
    /// 
    /// STORAGE_ACCOUNT
    /// STORAGE_ACCESS_KEY
    /// 
    /// TODO: Thorsten pls make this proper with *hand waving* managed identity and stuff!
    Environmental,
}

impl BlobStoreAzureBlob {
    pub fn new(
        // account: String,
        // container: String,
        auth_options: BlobStoreAzureAuthOptions,
    ) -> Result<Self> {
        let (account, credentials) = match auth_options {
            BlobStoreAzureAuthOptions::RuntimeConfigValues(config) => {
                (config.account.clone(), azure_storage::StorageCredentials::access_key(&config.account, config.key.clone()))
            },
            BlobStoreAzureAuthOptions::Environmental => {
                let account = std::env::var("STORAGE_ACCOUNT").expect("missing STORAGE_ACCOUNT");
                let access_key = std::env::var("STORAGE_ACCESS_KEY").expect("missing STORAGE_ACCOUNT_KEY");
                (account.clone(), azure_storage::StorageCredentials::access_key(account, access_key))
            },
        };

        let client = azure_storage_blobs::prelude::ClientBuilder::new(account, credentials).blob_service_client();
        Ok(Self { client })
    }
}

#[async_trait]
impl ContainerManager for BlobStoreAzureBlob {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error> {
        Ok(Arc::new(AzureBlobContainer {
            _name: name.to_owned(),
            client: self.client.container_client(name),
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }

    fn summary(&self, _store_name: &str) -> Option<String> {
        Some(format!("Azure blob storage account {}", self.client.account()))
    }
}

struct AzureBlobContainer {
    _name: String,
    client: ContainerClient,
}

/// Azure doesn't provide us with a container creation time
const DUMMY_CREATED_AT: u64 = 0;

#[async_trait]
impl Container for AzureBlobContainer {
    async fn exists(&self) -> anyhow::Result<bool> {
        Ok(self.client.exists().await?)
    }

    async fn name(&self) -> String {
        self.client.container_name().to_owned()
    }

    async fn info(&self) -> anyhow::Result<spin_factor_blobstore::ContainerMetadata> {
        let properties = self.client.get_properties().await?;
        Ok(spin_factor_blobstore::ContainerMetadata {
            name: properties.container.name,
            created_at: DUMMY_CREATED_AT,
        })
    }

    async fn clear(&self) -> anyhow::Result<()> {
        anyhow::bail!("Azure blob storage does not support clearing containers")
    }

    async fn delete_object(&self, name: &str) -> anyhow::Result<()> {
        self.client.blob_client(name).delete().await?;
        Ok(())
    }

    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()> {
        // TODO: are atomic semantics required? or efficiency guarantees?
        let futures = names.iter().map(|name| self.delete_object(name));
        futures::future::try_join_all(futures).await?;
        Ok(())
    }

    async fn has_object(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self.client.blob_client(name).exists().await?)
    }

    async fn object_info(&self, name: &str) -> anyhow::Result<spin_factor_blobstore::ObjectMetadata> {
        let response = self.client.blob_client(name).get_properties().await?;
        Ok(spin_factor_blobstore::ObjectMetadata {
            name: name.to_string(),
            container: self.client.container_name().to_string(),
            created_at: response.blob.properties.creation_time.unix_timestamp().try_into().unwrap(),
            size: response.blob.properties.content_length,
        })
    }

    async fn get_data(&self, name: &str, start: u64, end: u64) -> anyhow::Result<Box<dyn spin_factor_blobstore::IncomingData>> {
        // We can't use a Rust range because the Azure type does not accept inclusive ranges,
        // and we don't want to add 1 to `end` if it's already at MAX!
        let range = if end == u64::MAX {
            azure_core::request_options::Range::RangeFrom(start..)
        } else {
            azure_core::request_options::Range::Range(start..(end + 1))
        };
        let client = self.client.blob_client(name);
        Ok(Box::new(AzureBlobIncomingData::new(client, range)))
    }

    async fn list_objects(&self) -> anyhow::Result<Box<dyn spin_factor_blobstore::ObjectNames>> {
        let stm = self.client.list_blobs().into_stream();
        Ok(Box::new(AzureBlobBlobsList::new(stm)))
    }
}

struct AzureBlobIncomingData {
    // The Mutex is used to make it Send
    stm: Mutex<Option<
        azure_core::Pageable<
            azure_storage_blobs::blob::operations::GetBlobResponse,
            azure_core::error::Error
        >
    >>,
    client: azure_storage_blobs::prelude::BlobClient,
}

impl AzureBlobIncomingData {
    fn new(client: azure_storage_blobs::prelude::BlobClient, range: azure_core::request_options::Range) -> Self {
        let stm = client.get().range(range).into_stream();
        Self {
            stm: Mutex::new(Some(stm)),
            client,
        }
    }

    fn consume_async_impl(&mut self) -> wasmtime_wasi::pipe::AsyncReadStream { // Box<dyn futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>>> {
        use futures::TryStreamExt;
        use tokio_util::compat::FuturesAsyncReadCompatExt;
        let stm = self.consume_as_stream();
        let ar = stm.into_async_read();
        let arr = ar.compat();
        wasmtime_wasi::pipe::AsyncReadStream::new(arr)
        // Box::new(stm)
        // let async_read = stm.into_async_read();
        // todo!()
    }

    fn consume_as_stream(&mut self) -> impl futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>> {
        use futures::StreamExt;
        let opt_stm = self.stm.get_mut();
        let stm = opt_stm.take().unwrap();
        let byte_stm = stm.flat_map(|chunk| streamify_chunk(chunk.unwrap().data));
        byte_stm
    }
}

fn streamify_chunk(chunk: azure_core::ResponseBody) -> impl futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    use futures::StreamExt;
    chunk.map(|c| Ok(c.unwrap().to_vec()))
}


struct AzureBlobBlobsList {
    // The Mutex is used to make it Send
    stm: Mutex<
        azure_core::Pageable<
            azure_storage_blobs::container::operations::ListBlobsResponse,
            azure_core::error::Error
        >
    >,
    read_but_not_yet_returned: Vec<String>,
    end_stm_after_read_but_not_yet_returned: bool,
}

impl AzureBlobBlobsList {
    fn new(stm: azure_core::Pageable<
        azure_storage_blobs::container::operations::ListBlobsResponse,
        azure_core::error::Error
    >) -> Self {
        Self {
            stm: Mutex::new(stm),
            read_but_not_yet_returned: Default::default(),
            end_stm_after_read_but_not_yet_returned: false,
        }
    }

    async fn read_impl(&mut self, len: u64) -> anyhow::Result<(Vec<String>,bool)> {
        use futures::StreamExt;

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

        // TODO: do we need to prefix these with a prefix from somewhere or do they include it?
        let mut names: Vec<_> = chunk.blobs.blobs().map(|blob| blob.name.clone()).collect();
        let at_end = chunk.next_marker.is_none();

        if names.len() <= len {
            // We can send them all!
            return Ok((names, at_end));
        } else {
            // We have more names than we can send in this response. Send what we can and
            // stash the rest.
            let to_return: Vec<_> = names.drain(0..len).collect();
            self.read_but_not_yet_returned = names;
            self.end_stm_after_read_but_not_yet_returned = at_end;
            return Ok((to_return, false));
        }
    }
}

#[async_trait]
impl spin_factor_blobstore::IncomingData for AzureBlobIncomingData {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>> {
        use futures::StreamExt;
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

    fn consume_async(&mut self) -> wasmtime_wasi::pipe::AsyncReadStream { // Box<dyn futures::stream::Stream<Item = Result<Vec<u8>, std::io::Error>>> {
        self.consume_async_impl()
    }

    async fn size(&mut self) -> anyhow::Result<u64> {
        // TODO: in theory this should be infallible once we have the IncomingData
        // object. But in practice if we use the Pageable for that we don't get it until
        // we do the first read. So that would force us to either pre-fetch the
        // first chunk or to issue a properties request *just in case* size() was
        // called. So I'm making it fallible for now.
        Ok(self.client.get_properties().await?.blob.properties.content_length)
    }
}

#[async_trait]
impl spin_factor_blobstore::ObjectNames for AzureBlobBlobsList {
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
