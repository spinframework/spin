mod store;

use serde::Deserialize;
use spin_factor_blobstore::runtime_config::spin::MakeBlobStore;
use store::{
    BlobStoreAzureBlob,
    // KeyValueAzureCosmos, KeyValueAzureCosmosAuthOptions, KeyValueAzureCosmosRuntimeConfigOptions,
};

/// A key-value store that uses Azure Cosmos as the backend.
#[derive(Default)]
pub struct AzureBlobStore {
    _priv: (),
}

impl AzureBlobStore {
    /// Creates a new `AzureBlobStore`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Runtime configuration for the Azure Cosmos key-value store.
#[derive(Deserialize)]
pub struct AzureBlobStoreRuntimeConfig {
    /// The authorization token for the Azure blob storage  account.
    key: Option<String>,
    /// The Azure blob storage account name.
    account: String,
}

impl MakeBlobStore for AzureBlobStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "azure_blob";

    type RuntimeConfig = AzureBlobStoreRuntimeConfig;

    type ContainerManager = BlobStoreAzureBlob;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager> {
        let auth = match &runtime_config.key {
            Some(key) => store::BlobStoreAzureAuthOptions::RuntimeConfigValues(store::BlobStoreAzureRuntimeConfigOptions::new(runtime_config.account.clone(), key.clone())),
            None => store::BlobStoreAzureAuthOptions::Environmental,
        };
    

        // let account = &runtime_config.account;
        // let key = &runtime_config.key;

        // let credentials = azure_storage::prelude::StorageCredentials::access_key(account, key);
        // let client = azure_storage_blobs::prelude::ClientBuilder::new(account, credentials);

        let blob_store = BlobStoreAzureBlob::new(auth)?;
        Ok(blob_store)
    }
}
