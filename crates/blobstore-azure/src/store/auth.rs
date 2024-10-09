/// Azure blob storage runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct AzureKeyAuth {
    pub account: String,
    pub key: String,
}

impl AzureKeyAuth {
    pub fn new(account: String, key: String) -> Self {
        Self { account, key }
    }
}

/// Azure blob storage enumeration for the possible authentication options
#[derive(Clone, Debug)]
pub enum AzureBlobAuthOptions {
    /// The account and key have been specified directly
    AccountKey(AzureKeyAuth),
    /// Spin should use the environment variables of the process to
    /// create the StorageCredentials for the storage client. For now this uses old school credentials:
    ///
    /// STORAGE_ACCOUNT
    /// STORAGE_ACCESS_KEY
    ///
    /// TODO: Thorsten pls make this proper with *hand waving* managed identity and stuff!
    Environmental,
}
