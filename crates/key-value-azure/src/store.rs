use anyhow::Result;
use async_trait::async_trait;
use azure_core::credentials::Secret;
use azure_core::http::Etag;
use azure_data_cosmos::models::{PatchInstructions, PatchOperation};
use azure_data_cosmos::options::{ItemWriteOptions, Precondition, Region};
use azure_data_cosmos::{
    AccountReference, ContainerClient, CosmosClient, FeedScope, Query, RoutingStrategy,
};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use spin_factor_key_value::{
    Cas, Error, Store, StoreManager, SwapError, log_error, log_error_v3, v3,
};
use std::sync::{Arc, Mutex};

use crate::auth::KeyValueAzureCosmosAuthOptions;

pub struct KeyValueAzureCosmos {
    /// Parameters for initializing the Cosmos DB client
    account_ref: AccountReference,

    database: String,
    container: String,
    region: Region,

    /// The Cosmos DB client
    client: tokio::sync::OnceCell<ContainerClient>,
    /// An optional app id
    ///
    /// If provided, the store will handle multiple stores per container using a
    /// partition key of `/$app_id/$store_name`, otherwise there will be one container
    /// per store, and the partition key will be `/id`.
    app_id: Option<String>,
}

impl KeyValueAzureCosmos {
    pub fn new(
        account: String,
        database: String,
        container: String,
        auth_options: KeyValueAzureCosmosAuthOptions,
        region: Region,
        app_id: Option<String>,
    ) -> Result<Self> {
        let endpoint: azure_data_cosmos::AccountEndpoint =
            format!("https://{account}.documents.azure.com/")
                .parse()
                .map_err(log_error)?;

        let account_ref = match auth_options {
            KeyValueAzureCosmosAuthOptions::RuntimeConfigValues(config) => {
                AccountReference::with_authentication_key(
                    endpoint,
                    Secret::from(config.key.clone()),
                )
            }
            KeyValueAzureCosmosAuthOptions::AadCredential(kind) => {
                let credential = kind.credential().map_err(log_error)?;
                AccountReference::with_credential(endpoint, credential)
            }
        };

        Ok(Self {
            account_ref,
            database,
            container,
            region,
            client: tokio::sync::OnceCell::new(),
            app_id,
        })
    }
}

#[async_trait]
impl StoreManager for KeyValueAzureCosmos {
    async fn get(&self, name: &str) -> Result<Arc<dyn Store>, Error> {
        let client = self
            .client
            .get_or_try_init(|| async {
                return CosmosClient::builder()
                    .build(
                        self.account_ref.clone(),
                        RoutingStrategy::ProximityTo(self.region.clone()),
                    )
                    .await
                    .map_err(log_error)?
                    .database_client(&self.database)
                    .container_client(&self.container)
                    .await
                    .map_err(log_error);
            })
            .await?
            .clone();
        Ok(Arc::new(AzureCosmosStore {
            client,
            store_id: self.app_id.as_ref().map(|i| format!("{i}/{name}")),
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }

    fn summary(&self, _store_name: &str) -> Option<String> {
        Some(format!(
            "Azure CosmosDB database: {}, container: {}",
            self.database, self.container
        ))
    }
}

#[derive(Clone)]
struct AzureCosmosStore {
    client: ContainerClient,
    /// An optional store id to use as a partition key for all operations.
    ///
    /// If the store ID is not set, the store will use `/id` (the row key) as
    /// the partition key. For example, if `store.set("my_key", "my_value")` is
    /// called, the partition key will be `my_key` if the store ID is set to
    /// `None`. If the store ID is set to `Some("myappid/default"), the
    /// partition key will be `myappid/default`.
    store_id: Option<String>,
}

#[async_trait]
impl Store for AzureCosmosStore {
    async fn get(&self, key: &str, max_result_bytes: usize) -> Result<Option<Vec<u8>>, Error> {
        let partition_key = partition_key(self.store_id.as_deref(), key);
        let value = match self.client.read_item(partition_key, key, None).await {
            Ok(response) => Some(response.into_model::<Pair>().map_err(log_error)?.value),
            Err(e) if e.status().is_not_found() => None,
            Err(e) => return Err(log_error(e)),
        };

        if std::mem::size_of::<Option<Vec<u8>>>() + value.as_ref().map(Vec::len).unwrap_or(0)
            > max_result_bytes
        {
            Err(Error::Other(format!(
                "read result exceeds limit of {max_result_bytes} bytes"
            )))
        } else {
            Ok(value)
        }
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<(), Error> {
        let illegal_chars = ['/', '\\', '?', '#'];

        if key.contains(|c| illegal_chars.contains(&c)) {
            return Err(Error::Other(format!(
                "Key contains an illegal character. Keys must not include any of: {}",
                illegal_chars.iter().collect::<String>()
            )));
        }

        let pair = Pair {
            id: key.to_string(),
            value: value.to_vec(),
            store_id: self.store_id.clone(),
        };

        let partition_key = partition_key(self.store_id.as_deref(), key);
        self.client
            .upsert_item(partition_key, key, pair, None)
            .await
            .map_err(log_error)?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        let partition_key = partition_key(self.store_id.as_deref(), key);

        match self.client.delete_item(partition_key, key, None).await {
            Ok(_) => Ok(()),
            Err(e) if e.status().is_not_found() => Ok(()),
            Err(e) => Err(log_error(e)),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, Error> {
        let mut stream = self
            .client
            .query_items::<Key>(
                Query::from(self.get_id_query(key)),
                FeedScope::partition(partition_key(self.store_id.as_deref(), key)),
                None,
            )
            .await
            .map_err(log_error)?;
        Ok(stream.try_next().await.map_err(log_error)?.is_some())
    }

    async fn get_keys(&self, max_result_bytes: usize) -> Result<Vec<String>, Error> {
        let mut stream = self
            .client
            .query_items::<Key>(
                Query::from(self.get_keys_query()),
                FeedScope::full_container(),
                None,
            )
            .await
            .map_err(log_error)?;

        let mut result = Vec::new();
        let mut byte_count = std::mem::size_of::<Vec<String>>();

        while let Some(key) = stream.try_next().await.map_err(log_error)? {
            byte_count += std::mem::size_of::<String>() + key.id.len();
            if byte_count > max_result_bytes {
                return Err(Error::Other(format!(
                    "query result exceeds limit of {max_result_bytes} bytes"
                )));
            }
            result.push(key.id);
        }

        Ok(result)
    }

    async fn get_keys_async(
        &self,
        max_result_bytes: usize,
    ) -> (
        tokio::sync::mpsc::Receiver<String>,
        tokio::sync::oneshot::Receiver<Result<(), v3::Error>>,
    ) {
        let (keys_tx, keys_rx) = tokio::sync::mpsc::channel(4);
        let (err_tx, err_rx) = tokio::sync::oneshot::channel();

        let client = self.client.clone();
        let query = self.get_keys_query();

        let the_work = async move {
            let mut stream = client
                .query_items::<Key>(Query::from(query), FeedScope::full_container(), None)
                .await
                .map_err(log_error_v3)?;

            let mut byte_count = std::mem::size_of::<Vec<String>>();
            while let Some(key) = stream.try_next().await.map_err(log_error_v3)? {
                byte_count += std::mem::size_of::<String>() + key.id.len();
                if byte_count > max_result_bytes {
                    return Err(v3::Error::Other(format!(
                        "query exceeds limit of {max_result_bytes} bytes"
                    )));
                }

                keys_tx.send(key.id).await.map_err(log_error_v3)?;
            }
            Ok(())
        };
        tokio::spawn(async move {
            let res = the_work.await;
            _ = err_tx.send(res);
        });

        (keys_rx, err_rx)
    }

    async fn get_many(
        &self,
        keys: Vec<String>,
        max_result_bytes: usize,
    ) -> Result<Vec<(String, Option<Vec<u8>>)>, Error> {
        let mut stream = self
            .client
            .query_items::<Pair>(
                Query::from(self.get_in_query(keys)),
                FeedScope::full_container(),
                None,
            )
            .await
            .map_err(log_error)?;

        let mut results = Vec::new();
        let mut byte_count = std::mem::size_of::<Vec<(String, Option<Vec<u8>>)>>();
        while let Some(pair) = stream.try_next().await.map_err(log_error)? {
            byte_count +=
                std::mem::size_of::<(String, Option<Vec<u8>>)>() + pair.id.len() + pair.value.len();

            if byte_count > max_result_bytes {
                return Err(Error::Other(format!(
                    "query result exceeds limit of {max_result_bytes} bytes"
                )));
            }
            results.push((pair.id, Some(pair.value)))
        }
        Ok(results)
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

    /// Increments a numerical value.
    ///
    /// The initial value for the item must be set through this interface, as this sets the
    /// number value if it does not exist. If the value was previously set using
    /// the `set` interface, this will fail due to a type mismatch.
    async fn increment(&self, key: String, delta: i64) -> Result<i64, Error> {
        let patch =
            PatchInstructions::default().with_operation(PatchOperation::increment("/value", delta));
        let partition_key = partition_key(self.store_id.as_deref(), &key);

        match self
            .client
            .patch_item(partition_key.clone(), &key, patch, None)
            .await
        {
            Err(e) if e.status().is_not_found() => {
                let counter = Counter {
                    id: key.clone(),
                    value: delta,
                    store_id: self.store_id.clone(),
                };
                match self
                    .client
                    .create_item(partition_key, &key, counter, None)
                    .await
                {
                    Ok(_) => Ok(delta),
                    Err(e) if e.status().is_conflict() => self.increment(key, delta).await,
                    Err(e) => Err(log_error(e)),
                }
            }
            Err(e) => Err(log_error(e)),
            Ok(response) => Ok(response.into_model::<Counter>().map_err(log_error)?.value),
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
    client: ContainerClient,
    bucket_rep: u32,
    etag: Mutex<Option<Etag>>,
    store_id: Option<String>,
}

#[async_trait]
impl Cas for CompareAndSwap {
    /// `current` will fetch the current value for the key and store the etag for the record. The
    /// etag will be used to perform and optimistic concurrency update using the `if-match` header.
    async fn current(&self, max_result_bytes: usize) -> Result<Option<Vec<u8>>, Error> {
        let partition_key = partition_key(self.store_id.as_deref(), &self.key);
        let result = self.client.read_item(partition_key, &self.key, None).await;

        let value = match result {
            Ok(response) => {
                *self.etag.lock().unwrap() = response.headers().etag().cloned();
                Some(response.into_model::<Pair>().map_err(log_error)?.value)
            }
            Err(e) if e.status().is_not_found() => {
                *self.etag.lock().unwrap() = None;
                None
            }
            Err(e) => return Err(log_error(e)),
        };

        if std::mem::size_of::<Option<Vec<u8>>>() + value.as_ref().map(Vec::len).unwrap_or(0)
            > max_result_bytes
        {
            Err(Error::Other(format!(
                "query result exceeds limit of {max_result_bytes} bytes"
            )))
        } else {
            Ok(value)
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

        let partition_key = partition_key(self.store_id.as_deref(), &self.key);
        let etag = self.etag.lock().unwrap().clone();

        let response = match etag {
            Some(etag) => {
                let opts =
                    ItemWriteOptions::default().with_precondition(Precondition::IfMatch(etag));
                self.client
                    .replace_item(partition_key, &self.key, &pair, Some(opts))
                    .await
            }
            None => {
                self.client
                    .create_item(partition_key, &self.key, &pair, None)
                    .await
            }
        };

        response
            .map_err(|e| SwapError::CasFailed(format!("{e:?}")))
            .map(drop)
    }

    async fn bucket_rep(&self) -> u32 {
        self.bucket_rep
    }

    async fn key(&self) -> String {
        self.key.clone()
    }
}

impl AzureCosmosStore {
    fn get_id_query(&self, key: &str) -> String {
        let mut query = format!("SELECT c.id, c.store_id FROM c WHERE c.id='{key}'");
        append_store_id_condition(&mut query, self.store_id.as_deref(), true);
        query
    }

    fn get_keys_query(&self) -> String {
        let mut query = "SELECT c.id, c.store_id FROM c".to_owned();
        append_store_id_condition(&mut query, self.store_id.as_deref(), false);
        query
    }

    fn get_in_query(&self, keys: Vec<String>) -> String {
        let in_clause: String = keys
            .into_iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<String>>()
            .join(", ");

        let mut query = format!("SELECT * FROM c WHERE c.id IN ({in_clause})");
        append_store_id_condition(&mut query, self.store_id.as_deref(), true);
        query
    }
}

fn partition_key(store_id: Option<&str>, key: &str) -> String {
    store_id
        .map(str::to_string)
        .unwrap_or_else(|| key.to_string())
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

/// Pair structure for key value operations
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pair {
    pub id: String,
    pub value: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
}
/// Counter structure for increment operations
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Counter {
    pub id: String,
    pub value: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
}

/// Key structure for operations with generic value types
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Key {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
}
