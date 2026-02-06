use super::{Cas, SwapError};
use anyhow::{Context, Result};
use spin_core::{async_trait, wasmtime::component::Resource};
use spin_factor_otel::OtelFactorState;
use spin_resource_table::Table;
use spin_telemetry::traces::{self, Blame};
use spin_world::v2::key_value;
use spin_world::wasi::keyvalue as wasi_keyvalue;
use std::{collections::HashSet, sync::Arc};
use tracing::instrument;

const DEFAULT_STORE_TABLE_CAPACITY: u32 = 256;

pub use key_value::Error;

#[async_trait]
pub trait StoreManager: Sync + Send {
    async fn get(&self, name: &str) -> Result<Arc<dyn Store>, Error>;
    fn is_defined(&self, store_name: &str) -> bool;

    /// A human-readable summary of the given store's configuration
    ///
    /// Example: "Redis at localhost:1234"
    fn summary(&self, store_name: &str) -> Option<String> {
        let _ = store_name;
        None
    }
}

#[async_trait]
pub trait Store: Sync + Send {
    async fn after_open(&self) -> Result<(), Error> {
        Ok(())
    }
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, Error>;
    async fn set(&self, key: &str, value: &[u8]) -> Result<(), Error>;
    async fn delete(&self, key: &str) -> Result<(), Error>;
    async fn exists(&self, key: &str) -> Result<bool, Error>;
    async fn get_keys(&self) -> Result<Vec<String>, Error>;
    async fn get_many(&self, keys: Vec<String>) -> Result<Vec<(String, Option<Vec<u8>>)>, Error>;
    async fn set_many(&self, key_values: Vec<(String, Vec<u8>)>) -> Result<(), Error>;
    async fn delete_many(&self, keys: Vec<String>) -> Result<(), Error>;
    async fn increment(&self, key: String, delta: i64) -> Result<i64, Error>;
    async fn new_compare_and_swap(&self, bucket_rep: u32, key: &str)
        -> Result<Arc<dyn Cas>, Error>;
}

pub struct KeyValueDispatch {
    allowed_stores: HashSet<String>,
    manager: Arc<dyn StoreManager>,
    stores: Table<Arc<dyn Store>>,
    compare_and_swaps: Table<Arc<dyn Cas>>,
    otel: OtelFactorState,
}

impl KeyValueDispatch {
    pub fn new(allowed_stores: HashSet<String>, manager: Arc<dyn StoreManager>) -> Self {
        Self::new_with_capacity(
            allowed_stores,
            manager,
            DEFAULT_STORE_TABLE_CAPACITY,
            Default::default(),
        )
    }

    pub fn new_with_capacity(
        allowed_stores: HashSet<String>,
        manager: Arc<dyn StoreManager>,
        capacity: u32,
        otel: OtelFactorState,
    ) -> Self {
        Self {
            allowed_stores,
            manager,
            stores: Table::new(capacity),
            compare_and_swaps: Table::new(capacity),
            otel,
        }
    }

    pub fn get_store<T: 'static>(&self, store: Resource<T>) -> anyhow::Result<&Arc<dyn Store>> {
        let res = self.stores.get(store.rep()).context("invalid store");
        if let Err(err) = &res {
            traces::mark_as_error(err, Some(Blame::Host));
        }
        res
    }

    pub fn get_cas<T: 'static>(&self, cas: Resource<T>) -> Result<&Arc<dyn Cas>> {
        self.compare_and_swaps
            .get(cas.rep())
            .context("invalid compare and swap")
    }

    pub fn allowed_stores(&self) -> &HashSet<String> {
        &self.allowed_stores
    }

    pub fn get_store_wasi<T: 'static>(
        &self,
        store: Resource<T>,
    ) -> Result<&Arc<dyn Store>, wasi_keyvalue::store::Error> {
        self.stores
            .get(store.rep())
            .ok_or(wasi_keyvalue::store::Error::NoSuchStore)
    }

    pub fn get_cas_wasi<T: 'static>(
        &self,
        cas: Resource<T>,
    ) -> Result<&Arc<dyn Cas>, wasi_keyvalue::atomics::Error> {
        self.compare_and_swaps
            .get(cas.rep())
            .ok_or(wasi_keyvalue::atomics::Error::Other(
                "compare and swap not found".to_string(),
            ))
    }
}

impl key_value::Host for KeyValueDispatch {}

impl key_value::HostStore for KeyValueDispatch {
    #[instrument(name = "spin_key_value.open", skip(self), err, fields(otel.kind = "client", kv.backend=self.manager.summary(&name).unwrap_or("unknown".to_string())))]
    async fn open(&mut self, name: String) -> Result<Result<Resource<key_value::Store>, Error>> {
        self.otel.reparent_tracing_span();
        Ok(async {
            if self.allowed_stores.contains(&name) {
                let store = self.manager.get(&name).await?;
                store.after_open().await?;
                let store_idx = self
                    .stores
                    .push(store)
                    .map_err(|()| Error::StoreTableFull)?;
                Ok(Resource::new_own(store_idx))
            } else {
                Err(Error::AccessDenied)
            }
        }
        .await)
    }

    #[instrument(name = "spin_key_value.get", skip_all, fields(otel.kind = "client"))]
    async fn get(
        &mut self,
        store: Resource<key_value::Store>,
        key: String,
    ) -> Result<Result<Option<Vec<u8>>, Error>> {
        self.otel.reparent_tracing_span();
        let store = self.get_store(store)?;
        Ok(store.get(&key).await.map_err(track_error_on_span))
    }

    #[instrument(name = "spin_key_value.set", skip_all, fields(otel.kind = "client"))]
    async fn set(
        &mut self,
        store: Resource<key_value::Store>,
        key: String,
        value: Vec<u8>,
    ) -> Result<Result<(), Error>> {
        self.otel.reparent_tracing_span();
        let store = self.get_store(store)?;
        Ok(store.set(&key, &value).await.map_err(track_error_on_span))
    }

    #[instrument(name = "spin_key_value.delete", skip_all, fields(otel.kind = "client"))]
    async fn delete(
        &mut self,
        store: Resource<key_value::Store>,
        key: String,
    ) -> Result<Result<(), Error>> {
        self.otel.reparent_tracing_span();
        let store = self.get_store(store)?;
        Ok(store.delete(&key).await.map_err(track_error_on_span))
    }

    #[instrument(name = "spin_key_value.exists", skip_all, fields(otel.kind = "client"))]
    async fn exists(
        &mut self,
        store: Resource<key_value::Store>,
        key: String,
    ) -> Result<Result<bool, Error>> {
        self.otel.reparent_tracing_span();
        let store = self.get_store(store)?;
        Ok(store.exists(&key).await.map_err(track_error_on_span))
    }

    #[instrument(name = "spin_key_value.get_keys", skip_all, fields(otel.kind = "client"))]
    async fn get_keys(
        &mut self,
        store: Resource<key_value::Store>,
    ) -> Result<Result<Vec<String>, Error>> {
        self.otel.reparent_tracing_span();
        let store = self.get_store(store)?;
        Ok(store.get_keys().await.map_err(track_error_on_span))
    }

    async fn drop(&mut self, store: Resource<key_value::Store>) -> Result<()> {
        self.stores.remove(store.rep());
        Ok(())
    }
}

/// Make sure that infrastructure related errors are tracked in the current span.
fn track_error_on_span(err: Error) -> Error {
    let blame = match err {
        Error::NoSuchStore | Error::AccessDenied => Blame::Guest,
        Error::StoreTableFull | Error::Other(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    err
}

fn to_wasi_err(e: Error) -> wasi_keyvalue::store::Error {
    match track_error_on_span(e) {
        Error::AccessDenied => wasi_keyvalue::store::Error::AccessDenied,
        Error::NoSuchStore => wasi_keyvalue::store::Error::NoSuchStore,
        Error::StoreTableFull => wasi_keyvalue::store::Error::Other("store table full".to_string()),
        Error::Other(msg) => wasi_keyvalue::store::Error::Other(msg),
    }
}

impl wasi_keyvalue::store::Host for KeyValueDispatch {
    #[instrument(name = "wasi_key_value.open", skip_all, fields(otel.kind = "client"))]
    async fn open(
        &mut self,
        identifier: String,
    ) -> Result<Resource<wasi_keyvalue::store::Bucket>, wasi_keyvalue::store::Error> {
        if self.allowed_stores.contains(&identifier) {
            let store = self.manager.get(&identifier).await.map_err(to_wasi_err)?;
            store.after_open().await.map_err(to_wasi_err)?;
            let store_idx = self
                .stores
                .push(store)
                .map_err(|()| wasi_keyvalue::store::Error::Other("store table full".to_string()))?;
            Ok(Resource::new_own(store_idx))
        } else {
            Err(wasi_keyvalue::store::Error::AccessDenied)
        }
    }

    fn convert_error(
        &mut self,
        error: spin_world::wasi::keyvalue::store::Error,
    ) -> std::result::Result<spin_world::wasi::keyvalue::store::Error, anyhow::Error> {
        Ok(error)
    }
}

use wasi_keyvalue::store::Bucket;
impl wasi_keyvalue::store::HostBucket for KeyValueDispatch {
    #[instrument(name = "wasi_key_value.get", skip_all, fields(otel.kind = "client"))]
    async fn get(
        &mut self,
        self_: Resource<Bucket>,
        key: String,
    ) -> Result<Option<Vec<u8>>, wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(self_)?;
        store.get(&key).await.map_err(to_wasi_err)
    }

    #[instrument(name = "wasi_key_value.set", skip_all, fields(otel.kind = "client"))]
    async fn set(
        &mut self,
        self_: Resource<Bucket>,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(self_)?;
        store.set(&key, &value).await.map_err(to_wasi_err)
    }

    #[instrument(name = "wasi_key_value.delete", skip_all, fields(otel.kind = "client"))]
    async fn delete(
        &mut self,
        self_: Resource<Bucket>,
        key: String,
    ) -> Result<(), wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(self_)?;
        store.delete(&key).await.map_err(to_wasi_err)
    }

    #[instrument(name = "wasi_key_value.exists", skip_all, fields(otel.kind = "client"))]
    async fn exists(
        &mut self,
        self_: Resource<Bucket>,
        key: String,
    ) -> Result<bool, wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(self_)?;
        store.exists(&key).await.map_err(to_wasi_err)
    }

    #[instrument(name = "wasi_key_value.list_keys", skip_all, fields(otel.kind = "client"))]
    async fn list_keys(
        &mut self,
        self_: Resource<Bucket>,
        cursor: Option<String>,
    ) -> Result<wasi_keyvalue::store::KeyResponse, wasi_keyvalue::store::Error> {
        match cursor {
            Some(_) => Err(wasi_keyvalue::store::Error::Other(
                "list_keys: cursor not supported".to_owned(),
            )),
            None => {
                let store = self.get_store_wasi(self_)?;
                let keys = store.get_keys().await.map_err(to_wasi_err)?;
                Ok(wasi_keyvalue::store::KeyResponse { keys, cursor: None })
            }
        }
    }

    async fn drop(&mut self, rep: Resource<Bucket>) -> anyhow::Result<()> {
        self.stores.remove(rep.rep());
        Ok(())
    }
}

impl wasi_keyvalue::batch::Host for KeyValueDispatch {
    #[instrument(name = "spin_key_value.get_many", skip_all, fields(otel.kind = "client"))]
    #[allow(clippy::type_complexity)]
    async fn get_many(
        &mut self,
        bucket: Resource<wasi_keyvalue::batch::Bucket>,
        keys: Vec<String>,
    ) -> std::result::Result<Vec<(String, Option<Vec<u8>>)>, wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(bucket)?;
        if keys.is_empty() {
            return Ok(vec![]);
        }
        store.get_many(keys).await.map_err(to_wasi_err)
    }

    #[instrument(name = "spin_key_value.set_many", skip_all, fields(otel.kind = "client"))]
    async fn set_many(
        &mut self,
        bucket: Resource<wasi_keyvalue::batch::Bucket>,
        key_values: Vec<(String, Vec<u8>)>,
    ) -> std::result::Result<(), wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(bucket)?;
        if key_values.is_empty() {
            return Ok(());
        }
        store.set_many(key_values).await.map_err(to_wasi_err)
    }

    #[instrument(name = "spin_key_value.delete_many", skip_all, fields(otel.kind = "client"))]
    async fn delete_many(
        &mut self,
        bucket: Resource<wasi_keyvalue::batch::Bucket>,
        keys: Vec<String>,
    ) -> std::result::Result<(), wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(bucket)?;
        if keys.is_empty() {
            return Ok(());
        }
        store.delete_many(keys).await.map_err(to_wasi_err)
    }
}

impl wasi_keyvalue::atomics::HostCas for KeyValueDispatch {
    #[instrument(name = "wasi_key_value_cas.new", skip_all, fields(otel.kind = "client"))]
    async fn new(
        &mut self,
        bucket: Resource<wasi_keyvalue::atomics::Bucket>,
        key: String,
    ) -> Result<Resource<wasi_keyvalue::atomics::Cas>, wasi_keyvalue::store::Error> {
        let bucket_rep = bucket.rep();
        let bucket: Resource<Bucket> = Resource::new_own(bucket_rep);
        let store = self.get_store_wasi(bucket)?;
        let cas = store
            .new_compare_and_swap(bucket_rep, &key)
            .await
            .map_err(to_wasi_err)?;
        self.compare_and_swaps
            .push(cas)
            .map_err(|()| {
                spin_world::wasi::keyvalue::store::Error::Other(
                    "too many compare_and_swaps opened".to_string(),
                )
            })
            .map(Resource::new_own)
    }

    #[instrument(name = "wasi_key_value_cas.current", skip_all, fields(otel.kind = "client"))]
    async fn current(
        &mut self,
        cas: Resource<wasi_keyvalue::atomics::Cas>,
    ) -> Result<Option<Vec<u8>>, wasi_keyvalue::store::Error> {
        let cas = self
            .get_cas(cas)
            .map_err(|e| wasi_keyvalue::store::Error::Other(e.to_string()))?;
        cas.current().await.map_err(to_wasi_err)
    }

    async fn drop(&mut self, rep: Resource<wasi_keyvalue::atomics::Cas>) -> Result<()> {
        self.compare_and_swaps.remove(rep.rep());
        Ok(())
    }
}

impl wasi_keyvalue::atomics::Host for KeyValueDispatch {
    fn convert_cas_error(
        &mut self,
        error: spin_world::wasi::keyvalue::atomics::CasError,
    ) -> std::result::Result<spin_world::wasi::keyvalue::atomics::CasError, anyhow::Error> {
        Ok(error)
    }

    #[instrument(name = "spin_key_value.increment", skip_all, fields(otel.kind = "client"))]
    async fn increment(
        &mut self,
        bucket: Resource<wasi_keyvalue::atomics::Bucket>,
        key: String,
        delta: i64,
    ) -> Result<i64, wasi_keyvalue::store::Error> {
        let store = self.get_store_wasi(bucket)?;
        store.increment(key, delta).await.map_err(to_wasi_err)
    }

    #[instrument(name = "spin_key_value.swap", skip_all, fields(otel.kind = "client"))]
    async fn swap(
        &mut self,
        cas_res: Resource<atomics::Cas>,
        value: Vec<u8>,
    ) -> Result<(), CasError> {
        let cas_rep = cas_res.rep();
        let cas = self
            .get_cas(Resource::<Bucket>::new_own(cas_rep))
            .map_err(|e| CasError::StoreError(atomics::Error::Other(e.to_string())))?;

        match cas.swap(value).await {
            Ok(_) => Ok(()),
            Err(err) => match err {
                SwapError::CasFailed(_) => {
                    let bucket = Resource::new_own(cas.bucket_rep().await);
                    let new_cas = self
                        .new(bucket, cas.key().await)
                        .await
                        .map_err(CasError::StoreError)?;
                    let new_cas_rep = new_cas.rep();
                    self.current(Resource::new_own(new_cas_rep))
                        .await
                        .map_err(CasError::StoreError)?;
                    let res = Resource::new_own(new_cas_rep);
                    Err(CasError::CasFailed(res))
                }
                SwapError::Other(msg) => Err(CasError::StoreError(atomics::Error::Other(msg))),
            },
        }
    }
}

pub fn log_error(err: impl std::fmt::Debug) -> Error {
    tracing::warn!("key-value error: {err:?}");
    Error::Other(format!("{err:?}"))
}

pub fn log_cas_error(err: impl std::fmt::Debug) -> SwapError {
    tracing::warn!("key-value error: {err:?}");
    SwapError::Other(format!("{err:?}"))
}

use spin_world::v1::key_value::Error as LegacyError;
use spin_world::wasi::keyvalue::atomics;
use spin_world::wasi::keyvalue::atomics::{CasError, HostCas};

fn to_legacy_error(err: Error) -> LegacyError {
    match err {
        Error::StoreTableFull => LegacyError::StoreTableFull,
        Error::NoSuchStore => LegacyError::NoSuchStore,
        Error::AccessDenied => LegacyError::AccessDenied,
        Error::Other(s) => LegacyError::Io(s),
    }
}

impl spin_world::v1::key_value::Host for KeyValueDispatch {
    async fn open(&mut self, name: String) -> Result<Result<u32, LegacyError>> {
        let result = <Self as key_value::HostStore>::open(self, name).await?;
        Ok(result.map_err(to_legacy_error).map(|s| s.rep()))
    }

    async fn get(&mut self, store: u32, key: String) -> Result<Result<Vec<u8>, LegacyError>> {
        let this = Resource::new_borrow(store);
        let result = <Self as key_value::HostStore>::get(self, this, key).await?;
        Ok(result
            .map_err(to_legacy_error)
            .and_then(|v| v.ok_or(LegacyError::NoSuchKey)))
    }

    async fn set(
        &mut self,
        store: u32,
        key: String,
        value: Vec<u8>,
    ) -> Result<Result<(), LegacyError>> {
        let this = Resource::new_borrow(store);
        let result = <Self as key_value::HostStore>::set(self, this, key, value).await?;
        Ok(result.map_err(to_legacy_error))
    }

    async fn delete(&mut self, store: u32, key: String) -> Result<Result<(), LegacyError>> {
        let this = Resource::new_borrow(store);
        let result = <Self as key_value::HostStore>::delete(self, this, key).await?;
        Ok(result.map_err(to_legacy_error))
    }

    async fn exists(&mut self, store: u32, key: String) -> Result<Result<bool, LegacyError>> {
        let this = Resource::new_borrow(store);
        let result = <Self as key_value::HostStore>::exists(self, this, key).await?;
        Ok(result.map_err(to_legacy_error))
    }

    async fn get_keys(&mut self, store: u32) -> Result<Result<Vec<String>, LegacyError>> {
        let this = Resource::new_borrow(store);
        let result = <Self as key_value::HostStore>::get_keys(self, this).await?;
        Ok(result.map_err(to_legacy_error))
    }

    async fn close(&mut self, store: u32) -> Result<()> {
        let this = Resource::new_borrow(store);
        <Self as key_value::HostStore>::drop(self, this).await
    }
}
