use std::sync::Arc;

use anyhow::Result;
use azure_storage_blobs::prelude::{BlobServiceClient, ContainerClient};
use spin_core::async_trait;
use spin_factor_blobstore::{Container, ContainerManager, Error};

pub mod auth;
mod incoming_data;
mod object_names;

use auth::AzureBlobAuthOptions;
use incoming_data::AzureIncomingData;
use object_names::AzureObjectNames;

pub struct AzureContainerManager {
    client: BlobServiceClient,
}

impl AzureContainerManager {
    pub fn new(auth_options: AzureBlobAuthOptions) -> Result<Self> {
        let (account, credentials) = match auth_options {
            AzureBlobAuthOptions::AccountKey(config) => (
                config.account.clone(),
                azure_storage::StorageCredentials::access_key(&config.account, config.key.clone()),
            ),
            AzureBlobAuthOptions::Environmental => {
                let account = std::env::var("STORAGE_ACCOUNT").expect("missing STORAGE_ACCOUNT");
                let access_key =
                    std::env::var("STORAGE_ACCESS_KEY").expect("missing STORAGE_ACCOUNT_KEY");
                (
                    account.clone(),
                    azure_storage::StorageCredentials::access_key(account, access_key),
                )
            }
        };

        let client = azure_storage_blobs::prelude::ClientBuilder::new(account, credentials)
            .blob_service_client();
        Ok(Self { client })
    }
}

#[async_trait]
impl ContainerManager for AzureContainerManager {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error> {
        Ok(Arc::new(AzureContainer {
            _label: name.to_owned(),
            client: self.client.container_client(name),
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }
}

struct AzureContainer {
    _label: String,
    client: ContainerClient,
}

/// Azure doesn't provide us with a container creation time
const DUMMY_CREATED_AT: u64 = 0;

#[async_trait]
impl Container for AzureContainer {
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

    async fn object_info(
        &self,
        name: &str,
    ) -> anyhow::Result<spin_factor_blobstore::ObjectMetadata> {
        let response = self.client.blob_client(name).get_properties().await?;
        Ok(spin_factor_blobstore::ObjectMetadata {
            name: name.to_string(),
            container: self.client.container_name().to_string(),
            created_at: response
                .blob
                .properties
                .creation_time
                .unix_timestamp()
                .try_into()
                .unwrap(),
            size: response.blob.properties.content_length,
        })
    }

    async fn get_data(
        &self,
        name: &str,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Box<dyn spin_factor_blobstore::IncomingData>> {
        // We can't use a Rust range because the Azure type does not accept inclusive ranges,
        // and we don't want to add 1 to `end` if it's already at MAX!
        let range = if end == u64::MAX {
            azure_core::request_options::Range::RangeFrom(start..)
        } else {
            azure_core::request_options::Range::Range(start..(end + 1))
        };
        let client = self.client.blob_client(name);
        Ok(Box::new(AzureIncomingData::new(client, range)))
    }

    async fn write_data(
        &self,
        name: &str,
        data: tokio::io::ReadHalf<tokio::io::SimplexStream>,
        finished_tx: tokio::sync::mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        let client = self.client.blob_client(name);

        tokio::spawn(async move {
            let write_result = Self::write_data_core(data, client).await;
            finished_tx
                .send(write_result)
                .await
                .expect("should sent finish tx");
        });

        Ok(())
    }

    async fn list_objects(&self) -> anyhow::Result<Box<dyn spin_factor_blobstore::ObjectNames>> {
        let stm = self.client.list_blobs().into_stream();
        Ok(Box::new(AzureObjectNames::new(stm)))
    }
}

impl AzureContainer {
    async fn write_data_core(
        mut data: tokio::io::ReadHalf<tokio::io::SimplexStream>,
        client: azure_storage_blobs::prelude::BlobClient,
    ) -> anyhow::Result<()> {
        use tokio::io::AsyncReadExt;

        // Azure limits us to 50k blocks per blob.  At 2MB/block that allows 100GB, which will be
        // enough for most use cases.  If users need flexibility for larger blobs, we could make
        // the block size configurable via the runtime config ("size hint" or something).
        const BLOCK_SIZE: usize = 2 * 1024 * 1024;

        let mut blocks = vec![];

        'put_blocks: loop {
            let mut bytes = Vec::with_capacity(BLOCK_SIZE);
            loop {
                let read = data.read_buf(&mut bytes).await?;
                let len = bytes.len();

                if read == 0 {
                    // end of stream - send the last block and go
                    let id_bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
                    let block_id = azure_storage_blobs::prelude::BlockId::new(id_bytes);
                    client.put_block(block_id.clone(), bytes).await?;
                    blocks.push(azure_storage_blobs::blob::BlobBlockType::Uncommitted(
                        block_id,
                    ));
                    break 'put_blocks;
                }
                if len >= BLOCK_SIZE {
                    let id_bytes = uuid::Uuid::new_v4().as_bytes().to_vec();
                    let block_id = azure_storage_blobs::prelude::BlockId::new(id_bytes);
                    client.put_block(block_id.clone(), bytes).await?;
                    blocks.push(azure_storage_blobs::blob::BlobBlockType::Uncommitted(
                        block_id,
                    ));
                    break;
                }
            }
        }

        let block_list = azure_storage_blobs::blob::BlockList { blocks };
        client.put_block_list(block_list).await?;

        Ok(())
    }
}
