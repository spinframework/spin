use anyhow::{Context, Result};
use async_trait::async_trait;
use azure_data_cosmos::{
    prelude::{
        AuthorizationToken, CollectionClient, CosmosClient, CosmosClientBuilder, Operation, Query,
    },
    CosmosEntity,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use spin_factor_key_value::{log_cas_error, log_error, Cas, Error, Store, StoreManager, SwapError};
use std::sync::{Arc, Mutex};

pub struct KeyValueAzureCosmos {
    client: CollectionClient,
    /// An optional app id
    ///
    /// If provided, the store will handle multiple stores per container using a
    /// partition key of `/$app_id/$store_name`, otherwise there will be one container
    /// per store, and the partition key will be `/id`.
    app_id: Option<String>,
}

/// Azure Cosmos Key / Value runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct KeyValueAzureCosmosRuntimeConfigOptions {
    key: String,
}

impl KeyValueAzureCosmosRuntimeConfigOptions {
    pub fn new(key: String) -> Self {
        Self { key }
    }
}

/// Azure Cosmos Key / Value enumeration for the possible authentication options
#[derive(Clone, Debug)]
pub enum KeyValueAzureCosmosAuthOptions {
    /// Runtime Config values indicates the account and key have been specified directly
    RuntimeConfigValues(KeyValueAzureCosmosRuntimeConfigOptions),
    /// Environmental indicates that the environment variables of the process should be used to
    /// create the TokenCredential for the Cosmos client. This will use the Azure Rust SDK's
    /// DefaultCredentialChain to derive the TokenCredential based on what environment variables
    /// have been set.
    ///
    /// Service Principal with client secret:
    /// - `AZURE_TENANT_ID`: ID of the service principal's Azure tenant.
    /// - `AZURE_CLIENT_ID`: the service principal's client ID.
    /// - `AZURE_CLIENT_SECRET`: one of the service principal's secrets.
    ///
    /// Service Principal with certificate:
    /// - `AZURE_TENANT_ID`: ID of the service principal's Azure tenant.
    /// - `AZURE_CLIENT_ID`: the service principal's client ID.
    /// - `AZURE_CLIENT_CERTIFICATE_PATH`: path to a PEM or PKCS12 certificate file including the private key.
    /// - `AZURE_CLIENT_CERTIFICATE_PASSWORD`: (optional) password for the certificate file.
    ///
    /// Workload Identity (Kubernetes, injected by the Workload Identity mutating webhook):
    /// - `AZURE_TENANT_ID`: ID of the service principal's Azure tenant.
    /// - `AZURE_CLIENT_ID`: the service principal's client ID.
    /// - `AZURE_FEDERATED_TOKEN_FILE`: TokenFilePath is the path of a file containing a Kubernetes service account token.
    ///
    /// Managed Identity (User Assigned or System Assigned identities):
    /// - `AZURE_CLIENT_ID`: (optional) if using a user assigned identity, this will be the client ID of the identity.
    ///
    /// Azure CLI:
    /// - `AZURE_TENANT_ID`: (optional) use a specific tenant via the Azure CLI.
    ///
    /// Common across each:
    /// - `AZURE_AUTHORITY_HOST`: (optional) the host for the identity provider. For example, for Azure public cloud the host defaults to "https://login.microsoftonline.com".
    ///   See also: https://github.com/Azure/azure-sdk-for-rust/blob/main/sdk/identity/README.md
    Environmental,
}

impl KeyValueAzureCosmos {
    pub fn new(
        account: String,
        database: String,
        container: String,
        auth_options: KeyValueAzureCosmosAuthOptions,
        app_id: Option<String>,
    ) -> Result<Self> {
        let token = match auth_options {
            KeyValueAzureCosmosAuthOptions::RuntimeConfigValues(config) => {
                AuthorizationToken::primary_key(config.key).map_err(log_error)?
            }
            KeyValueAzureCosmosAuthOptions::Environmental => {
                AuthorizationToken::from_token_credential(
                    azure_identity::create_default_credential()?,
                )
            }
        };
        let cosmos_client = cosmos_client(account, token)?;
        let database_client = cosmos_client.database_client(database);
        let client = database_client.collection_client(container);

        Ok(Self { client, app_id })
    }
}

fn cosmos_client(account: impl Into<String>, token: AuthorizationToken) -> Result<CosmosClient> {
    if cfg!(feature = "connection-pooling") {
        let client = reqwest::ClientBuilder::new()
            .build()
            .context("failed to build reqwest client")?;
        let transport_options = azure_core::TransportOptions::new(std::sync::Arc::new(client));
        Ok(CosmosClientBuilder::new(account, token)
            .transport(transport_options)
            .build())
    } else {
        Ok(CosmosClient::new(account, token))
    }
}

#[async_trait]
impl StoreManager for KeyValueAzureCosmos {
    async fn get(&self, name: &str) -> Result<Arc<dyn Store>, Error> {
        Ok(Arc::new(AzureCosmosStore {
            client: self.client.clone(),
            store_id: self.app_id.as_ref().map(|i| format!("{i}/{name}")),
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }

    fn summary(&self, _store_name: &str) -> Option<String> {
        let database = self.client.database_client().database_name();
        let collection = self.client.collection_name();
        Some(format!(
            "Azure CosmosDB database: {database}, collection: {collection}"
        ))
    }
}

#[derive(Clone)]
struct AzureCosmosStore {
    client: CollectionClient,
    /// An optional store id to use as a partition key for all operations.
    ///
    /// If the store id not set, the store will use `/id` as the partition key.
    store_id: Option<String>,
}

#[async_trait]
impl Store for AzureCosmosStore {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let pair = self.get_pair(key).await?;
        Ok(pair.map(|p| p.value))
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<(), Error> {
        let pair = Pair {
            id: key.to_string(),
            value: value.to_vec(),
            store_id: self.store_id.clone(),
        };
        self.client
            .create_document(pair)
            .is_upsert(true)
            .await
            .map_err(log_error)?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        if self.exists(key).await? {
            let document_client = self
                .client
                .document_client(key, &self.store_id)
                .map_err(log_error)?;
            document_client.delete_document().await.map_err(log_error)?;
        }
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, Error> {
        Ok(self.get_pair(key).await?.is_some())
    }

    async fn get_keys(&self) -> Result<Vec<String>, Error> {
        self.get_keys().await
    }

    async fn get_many(&self, keys: Vec<String>) -> Result<Vec<(String, Option<Vec<u8>>)>, Error> {
        let stmt = Query::new(self.get_in_query(keys));
        let query = self
            .client
            .query_documents(stmt)
            .query_cross_partition(true);

        let mut res = Vec::new();
        let mut stream = query.into_stream::<Pair>();
        while let Some(resp) = stream.next().await {
            let resp = resp.map_err(log_error)?;
            res.extend(
                resp.results
                    .into_iter()
                    .map(|(pair, _)| (pair.id, Some(pair.value))),
            );
        }
        Ok(res)
    }

    async fn set_many(&self, key_values: Vec<(String, Vec<u8>)>) -> Result<(), Error> {
        for (key, value) in key_values {
            self.set(key.as_ref(), &value).await?
        }
        Ok(())
    }

    async fn delete_many(&self, keys: Vec<String>) -> Result<(), Error> {
        for key in keys {
            self.delete(key.as_ref()).await?
        }
        Ok(())
    }

    async fn increment(&self, key: String, delta: i64) -> Result<i64, Error> {
        let operations = vec![Operation::incr("/value", delta).map_err(log_error)?];
        let _ = self
            .client
            .document_client(key.clone(), &self.store_id)
            .map_err(log_error)?
            .patch_document(operations)
            .await
            .map_err(log_error)?;
        let pair = self.get_pair(key.as_ref()).await?;
        match pair {
            Some(p) => Ok(i64::from_le_bytes(
                p.value.try_into().expect("incorrect length"),
            )),
            None => Err(Error::Other(
                "increment returned an empty value after patching, which indicates a bug"
                    .to_string(),
            )),
        }
    }

    async fn new_compare_and_swap(
        &self,
        bucket_rep: u32,
        key: &str,
    ) -> Result<Arc<dyn spin_factor_key_value::Cas>, Error> {
        Ok(Arc::new(CompareAndSwap {
            key: key.to_string(),
            client: self.client.clone(),
            etag: Mutex::new(None),
            bucket_rep,
            store_id: self.store_id.clone(),
        }))
    }
}

struct CompareAndSwap {
    key: String,
    client: CollectionClient,
    bucket_rep: u32,
    etag: Mutex<Option<String>>,
    store_id: Option<String>,
}

impl CompareAndSwap {
    fn get_query(&self) -> String {
        let mut query = format!("SELECT * FROM c WHERE c.id='{}'", self.key);
        self.append_store_id(&mut query, true);
        query
    }

    fn append_store_id(&self, query: &mut String, condition_already_exists: bool) {
        append_store_id_condition(query, self.store_id.as_deref(), condition_already_exists);
    }
}

#[async_trait]
impl Cas for CompareAndSwap {
    /// `current` will fetch the current value for the key and store the etag for the record. The
    /// etag will be used to perform and optimistic concurrency update using the `if-match` header.
    async fn current(&self) -> Result<Option<Vec<u8>>, Error> {
        let mut stream = self
            .client
            .query_documents(Query::new(self.get_query()))
            .query_cross_partition(true)
            .max_item_count(1)
            .into_stream::<Pair>();

        let current_value: Option<(Vec<u8>, Option<String>)> = match stream.next().await {
            Some(r) => {
                let r = r.map_err(log_error)?;
                match r.results.first() {
                    Some((item, Some(attr))) => {
                        Some((item.clone().value, Some(attr.etag().to_string())))
                    }
                    Some((item, None)) => Some((item.clone().value, None)),
                    _ => None,
                }
            }
            None => None,
        };

        match current_value {
            Some((value, etag)) => {
                self.etag.lock().unwrap().clone_from(&etag);
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// `swap` updates the value for the key using the etag saved in the `current` function for
    /// optimistic concurrency.
    async fn swap(&self, value: Vec<u8>) -> Result<(), SwapError> {
        let pair = Pair {
            id: self.key.clone(),
            value,
            store_id: self.store_id.clone(),
        };

        let doc_client = self
            .client
            .document_client(&self.key, &pair.partition_key())
            .map_err(log_cas_error)?;

        let etag_value = self.etag.lock().unwrap().clone();
        match etag_value {
            Some(etag) => {
                // attempt to replace the document if the etag matches
                doc_client
                    .replace_document(pair)
                    .if_match_condition(azure_core::request_options::IfMatchCondition::Match(etag))
                    .await
                    .map_err(|e| SwapError::CasFailed(format!("{e:?}")))
                    .map(drop)
            }
            None => {
                // if we have no etag, then we assume the document does not yet exist and must insert; no upserts.
                self.client
                    .create_document(pair)
                    .await
                    .map_err(|e| SwapError::CasFailed(format!("{e:?}")))
                    .map(drop)
            }
        }
    }

    async fn bucket_rep(&self) -> u32 {
        self.bucket_rep
    }

    async fn key(&self) -> String {
        self.key.clone()
    }
}

impl AzureCosmosStore {
    async fn get_pair(&self, key: &str) -> Result<Option<Pair>, Error> {
        let query = self
            .client
            .query_documents(Query::new(self.get_query(key)))
            .query_cross_partition(true)
            .max_item_count(1);

        // There can be no duplicated keys, so we create the stream and only take the first result.
        let mut stream = query.into_stream::<Pair>();
        let Some(res) = stream.next().await else {
            return Ok(None);
        };
        Ok(res
            .map_err(log_error)?
            .results
            .first()
            .map(|(p, _)| p.clone()))
    }

    async fn get_keys(&self) -> Result<Vec<String>, Error> {
        let query = self
            .client
            .query_documents(Query::new(self.get_keys_query()))
            .query_cross_partition(true);
        let mut res = Vec::new();

        let mut stream = query.into_stream::<Pair>();
        while let Some(resp) = stream.next().await {
            let resp = resp.map_err(log_error)?;
            res.extend(resp.results.into_iter().map(|(pair, _)| pair.id));
        }

        Ok(res)
    }

    fn get_query(&self, key: &str) -> String {
        let mut query = format!("SELECT * FROM c WHERE c.id='{}'", key);
        self.append_store_id(&mut query, true);
        query
    }

    fn get_keys_query(&self) -> String {
        let mut query = "SELECT * FROM c".to_owned();
        self.append_store_id(&mut query, false);
        query
    }

    fn get_in_query(&self, keys: Vec<String>) -> String {
        let in_clause: String = keys
            .into_iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<String>>()
            .join(", ");

        let mut query = format!("SELECT * FROM c WHERE c.id IN ({})", in_clause);
        self.append_store_id(&mut query, true);
        query
    }

    fn append_store_id(&self, query: &mut String, condition_already_exists: bool) {
        append_store_id_condition(query, self.store_id.as_deref(), condition_already_exists);
    }
}

/// Appends an option store id condition to the query.
fn append_store_id_condition(
    query: &mut String,
    store_id: Option<&str>,
    condition_already_exists: bool,
) {
    if let Some(s) = store_id {
        if condition_already_exists {
            query.push_str(" AND");
        } else {
            query.push_str(" WHERE");
        }
        query.push_str(" c.store_id='");
        query.push_str(s);
        query.push('\'')
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pair {
    pub id: String,
    pub value: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
}

impl CosmosEntity for Pair {
    type Entity = String;

    fn partition_key(&self) -> Self::Entity {
        self.store_id.clone().unwrap_or_else(|| self.id.clone())
    }
}
