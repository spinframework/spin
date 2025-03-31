//! Example usage:
//!
//! --------------------
//!
//! spin.toml:
//!
//! [component.foo]
//! blob_containers = ["default"]
//!
//! --------------------
//!
//! runtime-config.toml
//!
//! [blob_store.default]
//! type = "file_system" | "s3" | "azure_blob"
//! # further config settings per type
//!
//! --------------------
//!
//! TODO: the naming here is not very consistent and we should make a more conscious
//! decision about whether these things are "blob stores" or "containers" or what

mod host;
pub mod runtime_config;
mod stream;
mod util;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use anyhow::ensure;
use spin_factors::{ConfigureAppContext, Factor, InitContext, PrepareContext, RuntimeFactors};
use spin_locked_app::MetadataKey;
use spin_resource_table::Table;

pub use host::{BlobStoreDispatch, Container, ContainerManager, Error, IncomingData, ObjectNames};
pub use runtime_config::RuntimeConfig;
pub use spin_world::wasi::blobstore::types::{ContainerMetadata, ObjectMetadata};
pub use stream::AsyncWriteStream;
use tokio::sync::RwLock;
pub use util::DelegatingContainerManager;

/// Lockfile metadata key for blob stores.
pub const BLOB_CONTAINERS_KEY: MetadataKey<Vec<String>> = MetadataKey::new("blob_containers");

/// A factor that provides blob storage.
#[derive(Default)]
pub struct BlobStoreFactor {
    _priv: (),
}

impl BlobStoreFactor {
    /// Create a new BlobStoreFactor.
    pub fn new() -> Self {
        Self { _priv: () }
    }
}

impl Factor for BlobStoreFactor {
    type RuntimeConfig = RuntimeConfig;
    type AppState = AppState;
    type InstanceBuilder = InstanceBuilder;

    fn init<T: Send + 'static>(&mut self, mut ctx: InitContext<T, Self>) -> anyhow::Result<()> {
        fn type_annotate<T, F>(f: F) -> F
        where
            F: Fn(&mut T) -> BlobStoreDispatch,
        {
            f
        }

        let get_data_with_table = ctx.get_data_with_table_fn();
        let closure = type_annotate(move |data| {
            let (state, table) = get_data_with_table(data);
            BlobStoreDispatch::new(
                &state.allowed_containers,
                state.container_manager.as_ref(),
                table,
                &state.containers,
                &state.incoming_values,
                &state.outgoing_values,
                &state.object_names,
            )
        });
        let linker = ctx.linker();

        spin_world::wasi::blobstore::blobstore::add_to_linker_get_host(linker, closure)?;
        spin_world::wasi::blobstore::container::add_to_linker_get_host(linker, closure)?;
        spin_world::wasi::blobstore::types::add_to_linker_get_host(linker, closure)?;

        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        mut ctx: ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        let runtime_config = ctx.take_runtime_config().unwrap_or_default();

        let delegating_manager = DelegatingContainerManager::new(runtime_config);
        let container_manager = Arc::new(delegating_manager);

        // Build component -> allowed containers map
        let mut component_allowed_containers = HashMap::new();
        for component in ctx.app().components() {
            let component_id = component.id().to_string();
            let containers = component
                .get_metadata(BLOB_CONTAINERS_KEY)?
                .unwrap_or_default()
                .into_iter()
                .collect::<HashSet<_>>();
            for label in &containers {
                ensure!(
                    container_manager.is_defined(label),
                    "unknown {} label {label:?} for component {component_id:?}",
                    BLOB_CONTAINERS_KEY.as_ref(),
                );
            }
            component_allowed_containers.insert(component_id, containers);
        }

        Ok(AppState {
            container_manager,
            component_allowed_containers,
        })
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<InstanceBuilder> {
        let app_state = ctx.app_state();
        let allowed_containers = app_state
            .component_allowed_containers
            .get(ctx.app_component().id())
            .expect("component should be in component_allowed_containers")
            .clone();
        let capacity = u32::MAX;
        Ok(InstanceBuilder {
            container_manager: app_state.container_manager.clone(),
            allowed_containers,
            containers: RwLock::new(Table::new(capacity)),
            incoming_values: RwLock::new(Table::new(capacity)),
            object_names: RwLock::new(Table::new(capacity)),
            outgoing_values: RwLock::new(Table::new(capacity)),
        })
    }
}

pub struct AppState {
    /// The container manager for the app.
    container_manager: Arc<DelegatingContainerManager>,
    /// The allowed containers for each component.
    ///
    /// This is a map from component ID to the set of container labels that the
    /// component is allowed to use.
    component_allowed_containers: HashMap<String, HashSet<String>>,
}

pub struct InstanceBuilder {
    /// The container manager for the app. This contains *all* container mappings.
    container_manager: Arc<DelegatingContainerManager>,
    /// The allowed containers for this component instance.
    allowed_containers: HashSet<String>,
    /// There are multiple WASI interfaces in play here. The factor adds each of them
    /// to the linker, passing a closure that derives the interface implementation
    /// from the InstanceBuilder.
    ///
    /// For the different interfaces to agree on their resource tables, each closure
    /// needs to derive the same resource table from the InstanceBuilder.
    /// So the InstanceBuilder (which is also the instance state) sets up all the resource
    /// tables and RwLocks them, then the dispatch object borrows them.
    containers: RwLock<Table<Arc<dyn Container>>>,
    incoming_values: RwLock<Table<Box<dyn IncomingData>>>,
    outgoing_values: RwLock<Table<host::OutgoingValue>>,
    object_names: RwLock<Table<Box<dyn ObjectNames>>>,
}

impl spin_factors::SelfInstanceBuilder for InstanceBuilder {}
