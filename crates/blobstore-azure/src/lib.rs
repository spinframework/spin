mod store;

use serde::Deserialize;
use spin_factor_blobstore::runtime_config::spin::MakeBlobStore;
use store::{
    auth::{AzureBlobAuthOptions, AzureKeyAuth},
    AzureContainerManager,
};

/// A key-value store that uses Azure Cosmos as the backend.
#[derive(Default)]
pub struct AzureBlobStoreBuilder {
    _priv: (),
}

impl AzureBlobStoreBuilder {
    /// Creates a new `AzureBlobStoreBuilder`.
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

impl MakeBlobStore for AzureBlobStoreBuilder {
    const RUNTIME_CONFIG_TYPE: &'static str = "azure_blob";

    type RuntimeConfig = AzureBlobStoreRuntimeConfig;

    type ContainerManager = AzureContainerManager;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager> {
        let auth = match &runtime_config.key {
            Some(key) => AzureBlobAuthOptions::AccountKey(AzureKeyAuth::new(
                runtime_config.account.clone(),
                key.clone(),
            )),
            None => AzureBlobAuthOptions::Environmental,
        };

        let blob_store = AzureContainerManager::new(auth)?;
        Ok(blob_store)
    }
}
