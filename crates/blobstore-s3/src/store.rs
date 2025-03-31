use std::sync::Arc;

use anyhow::Result;
use spin_core::async_trait;
use spin_factor_blobstore::{Container, ContainerManager, Error};

mod auth;
mod incoming_data;
mod object_names;

pub use auth::{S3AuthOptions, S3KeyAuth};
use incoming_data::S3IncomingData;
use object_names::S3ObjectNames;

pub struct S3ContainerManager {
    builder: object_store::aws::AmazonS3Builder,
    client: async_once_cell::Lazy<
        aws_sdk_s3::Client,
        std::pin::Pin<Box<dyn std::future::Future<Output = aws_sdk_s3::Client> + Send>>,
    >,
    bucket: Option<String>,
}

impl S3ContainerManager {
    pub fn new(
        region: String,
        auth_options: S3AuthOptions,
        bucket: Option<String>,
    ) -> Result<Self> {
        let builder = match &auth_options {
            S3AuthOptions::AccessKey(config) => object_store::aws::AmazonS3Builder::new()
                .with_region(&region)
                .with_access_key_id(&config.access_key)
                .with_secret_access_key(&config.secret_key)
                .with_token(config.token.clone().unwrap_or_default()),
            S3AuthOptions::Environmental => object_store::aws::AmazonS3Builder::from_env(),
        };

        let region_clone = region.clone();
        let client_fut = Box::pin(async move {
            let sdk_config = match auth_options {
                S3AuthOptions::AccessKey(config) => aws_config::SdkConfig::builder()
                    .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(
                        config,
                    ))
                    .region(aws_config::Region::new(region_clone))
                    .behavior_version(aws_config::BehaviorVersion::latest())
                    .build(),
                S3AuthOptions::Environmental => {
                    aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
                }
            };
            aws_sdk_s3::Client::new(&sdk_config)
        });

        Ok(Self {
            builder,
            client: async_once_cell::Lazy::from_future(client_fut),
            bucket,
        })
    }
}

#[async_trait]
impl ContainerManager for S3ContainerManager {
    async fn get(&self, name: &str) -> Result<Arc<dyn Container>, Error> {
        let name = self.bucket.clone().unwrap_or_else(|| name.to_owned());

        let store = self
            .builder
            .clone()
            .with_bucket_name(&name)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Arc::new(S3Container {
            name,
            store,
            client: self.client.get_unpin().await.clone(),
        }))
    }

    fn is_defined(&self, _store_name: &str) -> bool {
        true
    }
}

struct S3Container {
    name: String,
    store: object_store::aws::AmazonS3,
    client: aws_sdk_s3::Client,
}

/// S3 doesn't provide us with a container creation time
const DUMMY_CREATED_AT: u64 = 0;

#[async_trait]
impl Container for S3Container {
    async fn exists(&self) -> anyhow::Result<bool> {
        match self.client.head_bucket().bucket(&self.name).send().await {
            Ok(_) => Ok(true),
            Err(e) => match e.as_service_error() {
                Some(se) => Ok(!se.is_not_found()),
                None => anyhow::bail!(e),
            },
        }
    }

    async fn name(&self) -> String {
        self.name.clone()
    }

    async fn info(&self) -> anyhow::Result<spin_factor_blobstore::ContainerMetadata> {
        Ok(spin_factor_blobstore::ContainerMetadata {
            name: self.name.clone(),
            created_at: DUMMY_CREATED_AT,
        })
    }

    async fn clear(&self) -> anyhow::Result<()> {
        anyhow::bail!("AWS S3 blob storage does not support clearing containers")
    }

    async fn delete_object(&self, name: &str) -> anyhow::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.name)
            .key(name)
            .send()
            .await?;
        Ok(())
    }

    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()> {
        // TODO: are atomic semantics required? or efficiency guarantees?
        let futures = names.iter().map(|name| self.delete_object(name));
        futures::future::try_join_all(futures).await?;
        Ok(())
    }

    async fn has_object(&self, name: &str) -> anyhow::Result<bool> {
        match self
            .client
            .head_object()
            .bucket(&self.name)
            .key(name)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => match e.as_service_error() {
                Some(se) => Ok(!se.is_not_found()),
                None => anyhow::bail!(e),
            },
        }
    }

    async fn object_info(
        &self,
        name: &str,
    ) -> anyhow::Result<spin_factor_blobstore::ObjectMetadata> {
        let response = self
            .client
            .head_object()
            .bucket(&self.name)
            .key(name)
            .send()
            .await?;
        Ok(spin_factor_blobstore::ObjectMetadata {
            name: name.to_string(),
            container: self.name.clone(),
            created_at: response
                .last_modified()
                .and_then(|t| t.secs().try_into().ok())
                .unwrap_or(DUMMY_CREATED_AT),
            size: response
                .content_length
                .and_then(|l| l.try_into().ok())
                .unwrap_or_default(),
        })
    }

    async fn get_data(
        &self,
        name: &str,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Box<dyn spin_factor_blobstore::IncomingData>> {
        let range = if end == u64::MAX {
            format!("bytes={start}-")
        } else {
            format!("bytes={start}-{end}")
        };
        let resp = self
            .client
            .get_object()
            .bucket(&self.name)
            .key(name)
            .range(range)
            .send()
            .await?;
        Ok(Box::new(S3IncomingData::new(resp)))
    }

    async fn write_data(
        &self,
        name: &str,
        data: tokio::io::ReadHalf<tokio::io::SimplexStream>,
        finished_tx: tokio::sync::mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        let store = self.store.clone();
        let path = object_store::path::Path::from(name);

        tokio::spawn(async move {
            let write_result = Self::write_data_core(data, store, path).await;
            finished_tx
                .send(write_result)
                .await
                .expect("should sent finish tx");
        });

        Ok(())
    }

    async fn list_objects(&self) -> anyhow::Result<Box<dyn spin_factor_blobstore::ObjectNames>> {
        let stm = self
            .client
            .list_objects_v2()
            .bucket(&self.name)
            .into_paginator()
            .send();
        Ok(Box::new(S3ObjectNames::new(stm)))
    }
}

impl S3Container {
    async fn write_data_core(
        mut data: tokio::io::ReadHalf<tokio::io::SimplexStream>,
        store: object_store::aws::AmazonS3,
        path: object_store::path::Path,
    ) -> anyhow::Result<()> {
        use object_store::ObjectStore;

        const BUF_SIZE: usize = 5 * 1024 * 1024;

        let mupload = store.put_multipart(&path).await?;
        let mut writer = object_store::WriteMultipart::new(mupload);
        loop {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0; BUF_SIZE];
            let read_amount = data.read(&mut buf).await?;
            if read_amount == 0 {
                break;
            }
            buf.truncate(read_amount);
            writer.put(buf.into());
        }
        writer.finish().await?;

        Ok(())
    }
}
