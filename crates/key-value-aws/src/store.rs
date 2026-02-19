use core::str;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use aws_config::{BehaviorVersion, Region, SdkConfig};
use aws_credential_types::Credentials;
use aws_sdk_dynamodb::{
    config::{ProvideCredentials, SharedCredentialsProvider},
    operation::{
        batch_get_item::BatchGetItemOutput, batch_write_item::BatchWriteItemOutput,
        get_item::GetItemOutput,
    },
    primitives::Blob,
    types::{
        AttributeValue, DeleteRequest, KeysAndAttributes, PutRequest, TransactWriteItem, Update,
        WriteRequest,
    },
    Client,
};
use spin_core::async_trait;
use spin_factor_key_value::{log_error, Cas, Error, Store, StoreManager, SwapError};

pub struct KeyValueAwsDynamo {
    /// AWS region
    region: String,
    /// Whether to use strongly consistent reads
    consistent_read: bool,
    /// DynamoDB table, needs to be cloned when getting a store
    table: Arc<String>,
    /// DynamoDB client
    client: async_once_cell::Lazy<
        Client,
        std::pin::Pin<Box<dyn std::future::Future<Output = Client> + Send>>,
    >,
}

/// AWS Dynamo Key / Value runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct KeyValueAwsDynamoRuntimeConfigOptions {
    access_key: String,
    secret_key: String,
    token: Option<String>,
}

impl KeyValueAwsDynamoRuntimeConfigOptions {
    pub fn new(access_key: String, secret_key: String, token: Option<String>) -> Self {
        Self {
            access_key,
            secret_key,
            token,
        }
    }
}

impl ProvideCredentials for KeyValueAwsDynamoRuntimeConfigOptions {
    fn provide_credentials<'a>(
        &'a self,
    ) -> aws_credential_types::provider::future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        aws_credential_types::provider::future::ProvideCredentials::ready(Ok(Credentials::new(
            self.access_key.clone(),
            self.secret_key.clone(),
            self.token.clone(),
            None, // Optional expiration time
            "spin_custom_aws_provider",
        )))
    }
}

/// AWS Dynamo Key / Value enumeration for the possible authentication options
#[derive(Clone, Debug)]
pub enum KeyValueAwsDynamoAuthOptions {
    /// Runtime Config values indicates credentials have been specified directly
    RuntimeConfigValues(KeyValueAwsDynamoRuntimeConfigOptions),
    /// Environmental indicates that the environment variables of the process should be used to
    /// create the SDK Config for the Dynamo client. This will use the AWS Rust SDK's
    /// aws_config::load_defaults to derive credentials based on what environment variables
    /// have been set.
    ///
    /// See https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-authentication.html for options.
    Environmental,
}

impl KeyValueAwsDynamo {
    pub fn new(
        region: String,
        consistent_read: bool,
        table: String,
        auth_options: KeyValueAwsDynamoAuthOptions,
    ) -> Result<Self> {
        let region_clone = region.clone();
        let client_fut = Box::pin(async move {
            let sdk_config = match auth_options {
                KeyValueAwsDynamoAuthOptions::RuntimeConfigValues(config) => SdkConfig::builder()
                    .credentials_provider(SharedCredentialsProvider::new(config))
                    .region(Region::new(region_clone))
                    .behavior_version(BehaviorVersion::latest())
                    .build(),
                KeyValueAwsDynamoAuthOptions::Environmental => {
                    aws_config::load_defaults(BehaviorVersion::latest()).await
                }
            };
            Client::new(&sdk_config)
        });

        Ok(Self {
            region,
            consistent_read,
            table: Arc::new(table),
            client: async_once_cell::Lazy::from_future(client_fut),
        })
    }
}

#[async_trait]
impl StoreManager for KeyValueAwsDynamo {
    async fn get(&self, _name: &str) -> Result<Arc<dyn Store>, Error> {
        Ok(Arc::new(AwsDynamoStore {
            client: self.client.get_unpin().await.clone(),
            table: self.table.clone(),
            consistent_read: self.consistent_read,
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }

    fn summary(&self, _store_name: &str) -> Option<String> {
        Some(format!(
            "AWS DynamoDB region: {}, table: {}",
            self.region, self.table
        ))
    }
}

struct AwsDynamoStore {
    // Client wraps an Arc so should be low cost to clone
    client: Client,
    table: Arc<String>,
    consistent_read: bool,
}

#[derive(Debug, Clone)]
enum CasState {
    // Existing item with version
    Versioned(String),
    // Existing item without version
    Unversioned(Blob),
    // Item was missing when fetched during `current`, expected to be new
    Unset,
    // Potentially new item -- `current` was never called to fetch version
    Unknown,
}

struct CompareAndSwap {
    key: String,
    client: Client,
    table: Arc<String>,
    bucket_rep: u32,
    state: Mutex<CasState>,
}

/// Primary key in DynamoDB items used for querying items
const PK: &str = "PK";
/// Value key in DynamoDB items storing item value as binary
const VAL: &str = "VAL";
/// Version key in DynamoDB items used for atomic operations
const VER: &str = "VER";

#[async_trait]
impl Store for AwsDynamoStore {
    async fn get(&self, key: &str, max_result_bytes: usize) -> Result<Option<Vec<u8>>, Error> {
        let response = self
            .client
            .get_item()
            .consistent_read(self.consistent_read)
            .table_name(self.table.as_str())
            .key(
                PK,
                aws_sdk_dynamodb::types::AttributeValue::S(key.to_string()),
            )
            .projection_expression(VAL)
            .send()
            .await
            .map_err(log_error)?;

        let mut byte_count = std::mem::size_of::<Option<Vec<u8>>>();
        let item = response.item.and_then(|mut item| {
            if let Some(AttributeValue::B(val)) = item.remove(VAL) {
                let val = val.into_inner();
                byte_count += val.len();
                Some(val)
            } else {
                None
            }
        });

        // Currently there's no way to stream a `get_item` result without
        // buffering using `aws-sdk-dynamodb`, so the damage (in terms of host
        // memory usage) is already done, but we can still enforce the limit:
        if byte_count > max_result_bytes {
            return Err(Error::Other(format!(
                "query result exceeds limit of {max_result_bytes} bytes"
            )));
        }

        Ok(item)
    }

    async fn set(&self, key: &str, value: &[u8]) -> Result<(), Error> {
        self.client
            .put_item()
            .table_name(self.table.as_str())
            .item(PK, AttributeValue::S(key.to_string()))
            .item(VAL, AttributeValue::B(Blob::new(value)))
            .send()
            .await
            .map_err(log_error)?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        self.client
            .delete_item()
            .table_name(self.table.as_str())
            .key(PK, AttributeValue::S(key.to_string()))
            .send()
            .await
            .map_err(log_error)?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, Error> {
        let GetItemOutput { item, .. } = self
            .client
            .get_item()
            .consistent_read(self.consistent_read)
            .table_name(self.table.as_str())
            .key(
                PK,
                aws_sdk_dynamodb::types::AttributeValue::S(key.to_string()),
            )
            .projection_expression(PK)
            .send()
            .await
            .map_err(log_error)?;

        Ok(item.map(|item| item.contains_key(PK)).unwrap_or(false))
    }

    async fn get_keys(&self, max_result_bytes: usize) -> Result<Vec<String>, Error> {
        let mut primary_keys = Vec::new();

        let mut scan_paginator = self
            .client
            .scan()
            .table_name(self.table.as_str())
            .projection_expression(PK)
            .into_paginator()
            .send();

        let mut byte_count = std::mem::size_of::<Vec<String>>();
        while let Some(output) = scan_paginator.next().await {
            let scan_output = output.map_err(log_error)?;
            if let Some(items) = scan_output.items {
                for mut item in items {
                    if let Some(AttributeValue::S(pk)) = item.remove(PK) {
                        byte_count += std::mem::size_of::<String>() + pk.len();
                        if byte_count > max_result_bytes {
                            return Err(Error::Other(format!(
                                "query result exceeds limit of {max_result_bytes} bytes"
                            )));
                        }
                        primary_keys.push(pk);
                    }
                }
            }
        }

        Ok(primary_keys)
    }

    async fn get_many(
        &self,
        keys: Vec<String>,
        max_result_bytes: usize,
    ) -> Result<Vec<(String, Option<Vec<u8>>)>, Error> {
        let mut results = Vec::with_capacity(keys.len());
        let mut keys_and_attributes_builder = KeysAndAttributes::builder()
            .projection_expression(format!("{PK},{VAL}"))
            .consistent_read(self.consistent_read);
        for key in keys {
            keys_and_attributes_builder = keys_and_attributes_builder.keys(HashMap::from_iter([(
                PK.to_owned(),
                AttributeValue::S(key),
            )]))
        }
        let mut request_items = Some(HashMap::from_iter([(
            self.table.to_string(),
            keys_and_attributes_builder.build().map_err(log_error)?,
        )]));

        let mut byte_count = 0;
        while request_items.is_some() {
            let BatchGetItemOutput {
                responses,
                unprocessed_keys,
                ..
            } = self
                .client
                .batch_get_item()
                .set_request_items(request_items)
                .send()
                .await
                .map_err(log_error)?;

            if let Some(items) =
                responses.and_then(|mut responses| responses.remove(self.table.as_str()))
            {
                for mut item in items {
                    match (item.remove(PK), item.remove(VAL)) {
                        (Some(AttributeValue::S(pk)), Some(AttributeValue::B(val))) => {
                            let val = val.into_inner();
                            byte_count += std::mem::size_of::<(String, Option<Vec<u8>>)>()
                                + pk.len()
                                + val.len();
                            results.push((pk, Some(val)));
                        }
                        (Some(AttributeValue::S(pk)), None) => {
                            byte_count +=
                                std::mem::size_of::<(String, Option<Vec<u8>>)>() + pk.len();
                            results.push((pk, None));
                        }
                        _ => (),
                    }
                    if byte_count > max_result_bytes {
                        return Err(Error::Other(format!(
                            "query result exceeds limit of {max_result_bytes} bytes"
                        )));
                    }
                }
            }

            request_items = unprocessed_keys.filter(|unprocessed| !unprocessed.is_empty());
        }

        Ok(results)
    }

    async fn set_many(&self, key_values: Vec<(String, Vec<u8>)>) -> Result<(), Error> {
        let mut data = Vec::with_capacity(key_values.len());
        for (key, val) in key_values {
            data.push(
                WriteRequest::builder()
                    .put_request(
                        PutRequest::builder()
                            .item(PK, AttributeValue::S(key))
                            .item(VAL, AttributeValue::B(Blob::new(val)))
                            .build()
                            .map_err(log_error)?,
                    )
                    .build(),
            )
        }

        let mut request_items = Some(HashMap::from_iter([(self.table.to_string(), data)]));

        while request_items.is_some() {
            let BatchWriteItemOutput {
                unprocessed_items, ..
            } = self
                .client
                .batch_write_item()
                .set_request_items(request_items)
                .send()
                .await
                .map_err(log_error)?;

            request_items = unprocessed_items.filter(|unprocessed| !unprocessed.is_empty());
        }

        Ok(())
    }

    async fn delete_many(&self, keys: Vec<String>) -> Result<(), Error> {
        let mut data = Vec::with_capacity(keys.len());
        for key in keys {
            data.push(
                WriteRequest::builder()
                    .delete_request(
                        DeleteRequest::builder()
                            .key(PK, AttributeValue::S(key))
                            .build()
                            .map_err(log_error)?,
                    )
                    .build(),
            )
        }

        let mut request_items = Some(HashMap::from_iter([(self.table.to_string(), data)]));

        while request_items.is_some() {
            let BatchWriteItemOutput {
                unprocessed_items, ..
            } = self
                .client
                .batch_write_item()
                .set_request_items(request_items)
                .send()
                .await
                .map_err(log_error)?;

            request_items = unprocessed_items.filter(|unprocessed| !unprocessed.is_empty());
        }

        Ok(())
    }

    async fn increment(&self, key: String, delta: i64) -> Result<i64, Error> {
        let GetItemOutput { item, .. } = self
            .client
            .get_item()
            .consistent_read(true)
            .table_name(self.table.as_str())
            .key(PK, AttributeValue::S(key.clone()))
            .projection_expression(VAL)
            .send()
            .await
            .map_err(log_error)?;

        let old_val = match item {
            Some(mut current_item) => match current_item.remove(VAL) {
                // We're expecting i64, so technically we could transmute but seems risky...
                Some(AttributeValue::B(val)) => Some(
                    str::from_utf8(&val.into_inner())
                        .map_err(log_error)?
                        .parse::<i64>()
                        .map_err(log_error)?,
                ),
                _ => None,
            },
            None => None,
        };

        let new_val = old_val.unwrap_or(0) + delta;

        let mut update = Update::builder()
            .table_name(self.table.as_str())
            .key(PK, AttributeValue::S(key))
            .update_expression("SET #VAL = :new_val")
            .expression_attribute_names("#VAL", VAL)
            .expression_attribute_values(
                ":new_val",
                AttributeValue::B(Blob::new(new_val.to_string().as_bytes())),
            );

        if let Some(old_val) = old_val {
            update = update
                .condition_expression("#VAL = :old_val")
                .expression_attribute_values(
                    ":old_val",
                    AttributeValue::B(Blob::new(old_val.to_string().as_bytes())),
                )
        } else {
            update = update.condition_expression("attribute_not_exists (#VAL)")
        }

        self.client
            .transact_write_items()
            .transact_items(
                TransactWriteItem::builder()
                    .update(update.build().map_err(log_error)?)
                    .build(),
            )
            .send()
            .await
            .map_err(log_error)?;

        Ok(new_val)
    }

    async fn new_compare_and_swap(
        &self,
        bucket_rep: u32,
        key: &str,
    ) -> Result<Arc<dyn spin_factor_key_value::Cas>, Error> {
        Ok(Arc::new(CompareAndSwap {
            key: key.to_string(),
            client: self.client.clone(),
            table: self.table.clone(),
            state: Mutex::new(CasState::Unknown),
            bucket_rep,
        }))
    }
}

#[async_trait]
impl Cas for CompareAndSwap {
    async fn current(&self, max_result_bytes: usize) -> Result<Option<Vec<u8>>, Error> {
        let GetItemOutput { item, .. } = self
            .client
            .get_item()
            .consistent_read(true)
            .table_name(self.table.as_str())
            .key(PK, AttributeValue::S(self.key.clone()))
            .projection_expression(format!("{VAL},{VER}"))
            .send()
            .await
            .map_err(log_error)?;

        let mut byte_count = std::mem::size_of::<Option<Vec<u8>>>();
        let value = match item {
            Some(mut current_item) => match (current_item.remove(VAL), current_item.remove(VER)) {
                (Some(AttributeValue::B(val)), Some(AttributeValue::N(ver))) => {
                    let val = val.into_inner();
                    byte_count += val.len();

                    self.state
                        .lock()
                        .unwrap()
                        .clone_from(&CasState::Versioned(ver));

                    Some(val)
                }
                (Some(AttributeValue::B(val)), _) => {
                    self.state
                        .lock()
                        .unwrap()
                        .clone_from(&CasState::Unversioned(val.clone()));

                    let val = val.into_inner();
                    byte_count += val.len();

                    Some(val)
                }
                (_, _) => {
                    self.state.lock().unwrap().clone_from(&CasState::Unset);
                    None
                }
            },
            None => {
                self.state.lock().unwrap().clone_from(&CasState::Unset);
                None
            }
        };

        // Currently there's no way to stream a `get_item` result without
        // buffering using `aws-sdk-dynamodb`, so the damage (in terms of host
        // memory usage) is already done, but we can still enforce the limit:
        if byte_count > max_result_bytes {
            return Err(Error::Other(format!(
                "query result exceeds limit of {max_result_bytes} bytes"
            )));
        }

        Ok(value)
    }

    /// `swap` updates the value for the key -- if possible, using the version saved in the `current` function for
    /// optimistic concurrency or the previous item value
    async fn swap(&self, value: Vec<u8>) -> Result<(), SwapError> {
        let mut update = Update::builder()
            .table_name(self.table.as_str())
            .key(PK, AttributeValue::S(self.key.clone()))
            .update_expression("SET #VAL = :val ADD #VER :increment")
            .expression_attribute_names("#VAL", VAL)
            .expression_attribute_names("#VER", VER)
            .expression_attribute_values(":val", AttributeValue::B(Blob::new(value)))
            .expression_attribute_values(":increment", AttributeValue::N("1".to_owned()));

        let state = self.state.lock().unwrap().clone();
        match state {
            CasState::Versioned(version) => {
                update = update
                    .condition_expression("#VER = :ver")
                    .expression_attribute_values(":ver", AttributeValue::N(version));
            }
            CasState::Unversioned(old_val) => {
                update = update
                    .condition_expression("#VAL = :old_val")
                    .expression_attribute_values(":old_val", AttributeValue::B(old_val));
            }
            CasState::Unset => {
                update = update.condition_expression("attribute_not_exists (#VAL)");
            }
            CasState::Unknown => (),
        };

        self.client
            .transact_write_items()
            .transact_items(
                TransactWriteItem::builder()
                    .update(
                        update
                            .build()
                            .map_err(|e| SwapError::Other(format!("{e:?}")))?,
                    )
                    .build(),
            )
            .send()
            .await
            .map_err(|e| SwapError::CasFailed(format!("{e:?}")))?;

        Ok(())
    }

    async fn bucket_rep(&self) -> u32 {
        self.bucket_rep
    }

    async fn key(&self) -> String {
        self.key.clone()
    }
}
