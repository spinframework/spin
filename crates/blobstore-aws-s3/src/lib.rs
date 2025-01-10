mod store;

use serde::Deserialize;
use spin_factor_blobstore::runtime_config::spin::MakeBlobStore;
use store::BlobStoreS3;

/// A key-value store that uses Azure Cosmos as the backend.
#[derive(Default)]
pub struct S3BlobStore {
    _priv: (),
}

impl S3BlobStore {
    /// Creates a new `AzureBlobStore`.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Runtime configuration for the Azure Cosmos key-value store.
#[derive(Deserialize)]
pub struct S3BlobStoreRuntimeConfig {
    /// The access key for the AWS S3 account role.
    access_key: Option<String>,
    /// The secret key for authorization on the AWS S3 account.
    secret_key: Option<String>,
    /// The token for authorization on the AWS S3 account.
    token: Option<String>,
    /// The AWS region where the S3 account is located
    region: String,
}

impl MakeBlobStore for S3BlobStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "aws_s3";

    type RuntimeConfig = S3BlobStoreRuntimeConfig;

    type ContainerManager = BlobStoreS3;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager> {
        let auth = match (&runtime_config.access_key, &runtime_config.secret_key) {
            (Some(access_key), Some(secret_key)) => store::BlobStoreS3AuthOptions::RuntimeConfigValues(store::BlobStoreS3RuntimeConfigOptions::new(access_key.clone(), secret_key.clone(), runtime_config.token.clone())),
            (None, None) => store::BlobStoreS3AuthOptions::Environmental,
            _ => anyhow::bail!("either both of access_key and secret_key must be provided, or neither"),
        };
    
        let blob_store = BlobStoreS3::new(runtime_config.region, auth)?;
        Ok(blob_store)
    }
}
