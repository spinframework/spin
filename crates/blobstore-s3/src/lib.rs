mod store;

use serde::Deserialize;
use spin_factor_blobstore::runtime_config::spin::MakeBlobStore;
use store::S3ContainerManager;

/// A blob store that uses a S3-compatible service as the backend.
/// This currently supports only AWS S3
#[derive(Default)]
pub struct S3BlobStore {
    _priv: (),
}

impl S3BlobStore {
    /// Creates a new `S3BlobStore`.
    pub fn new() -> Self {
        Self::default()
    }
}

// TODO: allow URL configuration for compatible non-AWS services

/// Runtime configuration for the S3 blob store.
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
    /// The name of the bucket backing the store. The default is the store label.
    bucket: Option<String>,
}

impl MakeBlobStore for S3BlobStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "s3";

    type RuntimeConfig = S3BlobStoreRuntimeConfig;

    type ContainerManager = S3ContainerManager;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager> {
        let auth = match (&runtime_config.access_key, &runtime_config.secret_key) {
            (Some(access_key), Some(secret_key)) => {
                store::S3AuthOptions::AccessKey(store::S3KeyAuth::new(
                    access_key.clone(),
                    secret_key.clone(),
                    runtime_config.token.clone(),
                ))
            }
            (None, None) => store::S3AuthOptions::Environmental,
            _ => anyhow::bail!(
                "either both of access_key and secret_key must be provided, or neither"
            ),
        };

        let blob_store =
            S3ContainerManager::new(runtime_config.region, auth, runtime_config.bucket)?;
        Ok(blob_store)
    }
}
