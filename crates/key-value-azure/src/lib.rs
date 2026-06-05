mod auth;
mod store;

use azure_data_cosmos::Region;
use serde::Deserialize;
use spin_factor_key_value::runtime_config::spin::MakeKeyValueStore;

pub use auth::{
    AzureCredentialKind, KeyValueAzureCosmosAuthOptions, KeyValueAzureCosmosRuntimeConfigOptions,
};
pub use store::KeyValueAzureCosmos;

/// A key-value store that uses Azure Cosmos as the backend.
pub struct AzureKeyValueStore {
    app_id: Option<String>,
}

impl AzureKeyValueStore {
    /// Creates a new `AzureKeyValueStore`.
    ///
    /// When `app_id` is provided, the store will use a partition key of `$app_id/$store_name`,
    /// otherwise the partition key will be `id`.
    pub fn new(app_id: Option<String>) -> Self {
        Self { app_id }
    }
}

/// Runtime configuration for the Azure Cosmos key-value store.
#[derive(Deserialize)]
pub struct AzureCosmosKeyValueRuntimeConfig {
    /// The authorization token for the Azure Cosmos DB account.
    key: Option<String>,
    /// The Azure Cosmos DB account name.
    account: String,
    /// The Azure Cosmos DB database.
    database: String,
    /// The Azure Cosmos DB container where data is stored.
    /// The container's partition key path must be `/id` (the default) — or
    /// `/store_id` if the store is constructed with an `app_id`.
    container: String,

    /// Optional. The Azure region the spin application is running in (or the
    /// closest Azure region to it), used as the proximity-sorting anchor
    /// for the Azure SDK's region selection. When omitted, defaults to
    /// East US.
    region: Option<String>,

    /// Optional. When `key` is omitted, selects which Azure AD credential to
    /// use: "managed_identity", "workload_identity", "service_principal", or
    /// "developer_tools". When omitted, defaults to developer tools (Azure CLI
    /// / azd), intended for local development. Ignored when `key` is set.
    ///
    /// "service_principal" reads `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, and
    /// `AZURE_CLIENT_SECRET` from the environment.
    ///
    /// There is intentionally no automatic fallback between credential types;
    /// name the one matching your deployment.
    auth_type: Option<String>,

    /// Optional. Only used with `auth_type = "managed_identity"`: the client ID
    /// of a user-assigned managed identity to authenticate. When omitted, the
    /// system-assigned identity is used. Ignored with any other `auth_type`.
    client_id: Option<String>,
}

impl MakeKeyValueStore for AzureKeyValueStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "azure_cosmos";

    type RuntimeConfig = AzureCosmosKeyValueRuntimeConfig;

    type StoreManager = KeyValueAzureCosmos;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::StoreManager> {
        let auth_options = match runtime_config.key {
            Some(key) => KeyValueAzureCosmosAuthOptions::RuntimeConfigValues(
                KeyValueAzureCosmosRuntimeConfigOptions::new(key),
            ),
            None => {
                KeyValueAzureCosmosAuthOptions::AadCredential(AzureCredentialKind::from_auth_type(
                    runtime_config.auth_type.as_deref(),
                    runtime_config.client_id,
                )?)
            }
        };
        let region = runtime_config
            .region
            .map(Region::from)
            .unwrap_or(Region::EAST_US);
        KeyValueAzureCosmos::new(
            runtime_config.account,
            runtime_config.database,
            runtime_config.container,
            auth_options,
            region,
            self.app_id.clone(),
        )
    }
}
