mod host;
pub mod runtime_config;
mod util;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::ensure;
use spin_factors::{
    ConfigureAppContext, Factor, FactorInstanceBuilder, InitContext, PrepareContext, RuntimeFactors,
};
use spin_locked_app::MetadataKey;

/// Metadata key for key-value stores.
pub const KEY_VALUE_STORES_KEY: MetadataKey<Vec<String>> = MetadataKey::new("key_value_stores");
pub use host::{log_cas_error, log_error, Error, KeyValueDispatch, Store, StoreManager};
pub use runtime_config::RuntimeConfig;
use spin_core::async_trait;
pub use util::DelegatingStoreManager;

/// A factor that provides key-value storage.
#[derive(Default)]
pub struct KeyValueFactor {
    _priv: (),
}

impl KeyValueFactor {
    /// Create a new KeyValueFactor.
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Factor for KeyValueFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceBuilder;

    fn init<C>(&mut self, ctx: &mut C) -> anyhow::Result<()>
    where
        C: InitContext<Self>,
    {
        ctx.link_bindings(spin_world::v1::key_value::add_to_linker)?;
        ctx.link_bindings(spin_world::v2::key_value::add_to_linker)?;
        ctx.link_bindings(spin_world::wasi::keyvalue::store::add_to_linker)?;
        ctx.link_bindings(spin_world::wasi::keyvalue::batch::add_to_linker)?;
        ctx.link_bindings(spin_world::wasi::keyvalue::atomics::add_to_linker)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let store_managers = ctx.take_runtime_config().unwrap_or_default();

        let delegating_manager = DelegatingStoreManager::new(store_managers);
        let store_manager = Arc::new(delegating_manager);

        // Build component -> allowed stores map
        let mut component_allowed_stores = HashMap::new();
        for component in ctx.app().components() {
            let component_id = component.id().to_string();
            let key_value_stores = component
                .get_metadata(KEY_VALUE_STORES_KEY)?
                .unwrap_or_default()
                .into_iter()
                .collect::<HashSet<_>>();
            for label in &key_value_stores {
                // TODO: port nicer errors from KeyValueComponent (via error type?)
                ensure!(
                    store_manager.is_defined(label),
                    "unknown key_value_stores label {label:?} for component {component_id:?}"
                );
            }
            component_allowed_stores.insert(component_id, key_value_stores);
            // TODO: warn (?) on unused store?
        }

        Ok(AppState {
            store_manager,
            component_allowed_stores,
        })
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<InstanceBuilder> {
        let app_state = ctx.app_state();
        let allowed_stores = app_state
            .component_allowed_stores
            .get(ctx.app_component().id())
            .expect("component should be in component_stores")
            .clone();
        Ok(InstanceBuilder {
            store_manager: app_state.store_manager.clone(),
            allowed_stores,
        })
    }
}

type AppStoreManager = DelegatingStoreManager;

pub struct AppState {
    /// The store manager for the app.
    ///
    /// This is a cache around a delegating store manager. For `get` requests,
    /// first checks the cache before delegating to the underlying store
    /// manager.
    store_manager: Arc<AppStoreManager>,
    /// The allowed stores for each component.
    ///
    /// This is a map from component ID to the set of store labels that the
    /// component is allowed to use.
    component_allowed_stores: HashMap<String, HashSet<String>>,
}

impl AppState {
    /// Returns the [`StoreManager::summary`] for the given store label.
    pub fn store_summary(&self, label: &str) -> Option<String> {
        self.store_manager.summary(label)
    }

    /// Returns true if the given store label is used by any component.
    pub fn store_is_used(&self, label: &str) -> bool {
        self.component_allowed_stores
            .values()
            .any(|stores| stores.contains(label))
    }

    /// Get a store by label.
    pub async fn get_store(&self, label: &str) -> Option<Arc<dyn Store>> {
        self.store_manager.get(label).await.ok()
    }
}

/// `SwapError` are errors that occur during compare and swap operations
#[derive(Debug, thiserror::Error)]
pub enum SwapError {
    #[error("{0}")]
    CasFailed(String),

    #[error("{0}")]
    Other(String),
}

/// `Cas` trait describes the interface a key value compare and swap implementor must fulfill.
///
/// `current` is expected to get the current value for the key associated with the CAS operation
/// while also starting what is needed to ensure the value to be replaced will not have mutated
/// between the time of calling `current` and `swap`. For example, a get from a backend store
/// may provide the caller with an etag (a version stamp), which can be used with an if-match
/// header to ensure the version updated is the version that was read (optimistic concurrency).
/// Rather than an etag, one could start a transaction, if supported by the backing store, which
/// would provide atomicity.
///
/// `swap` is expected to replace the old value with the new value respecting the atomicity of the
/// operation. If there was no key / value with the given key in the store, the `swap` operation
/// should **insert** the key and value, disallowing an update.
#[async_trait]
pub trait Cas: Sync + Send {
    async fn current(&self) -> anyhow::Result<Option<Vec<u8>>, Error>;
    async fn swap(&self, value: Vec<u8>) -> anyhow::Result<(), SwapError>;
    async fn bucket_rep(&self) -> u32;
    async fn key(&self) -> String;
}

pub struct InstanceBuilder {
    /// The store manager for the app.
    ///
    /// This is a cache around a delegating store manager. For `get` requests,
    /// first checks the cache before delegating to the underlying store
    /// manager.
    store_manager: Arc<AppStoreManager>,
    /// The allowed stores for this component instance.
    allowed_stores: HashSet<String>,
}

impl FactorInstanceBuilder for InstanceBuilder {
    type InstanceState = KeyValueDispatch;

    fn build(self) -> anyhow::Result<Self::InstanceState> {
        let Self {
            store_manager,
            allowed_stores,
        } = self;
        Ok(KeyValueDispatch::new_with_capacity(
            allowed_stores,
            store_manager,
            u32::MAX,
        ))
    }
}
