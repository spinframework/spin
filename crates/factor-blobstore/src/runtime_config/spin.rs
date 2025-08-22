//! Runtime configuration implementation used by Spin CLI.

use crate::{ContainerManager, RuntimeConfig};
use anyhow::Context as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use spin_factors::runtime_config::toml::GetTomlValue;
use std::{collections::HashMap, sync::Arc};

/// Defines the construction of a blob store from a serialized runtime config.
pub trait MakeBlobStore: 'static + Send + Sync {
    /// Unique type identifier for the store.
    const RUNTIME_CONFIG_TYPE: &'static str;
    /// Runtime configuration for the store.
    type RuntimeConfig: DeserializeOwned;
    /// The store manager for the store.
    type ContainerManager: ContainerManager;

    /// Creates a new store manager from the runtime configuration.
    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager>;
}

/// A function that creates a container manager from a TOML table.
type StoreFromToml =
    Arc<dyn Fn(toml::Table) -> anyhow::Result<Arc<dyn ContainerManager>> + Send + Sync>;

/// Creates a `StoreFromToml` function from a `MakeBlobStore` implementation.
fn store_from_toml_fn<T: MakeBlobStore>(provider_type: T) -> StoreFromToml {
    Arc::new(move |table| {
        let runtime_config: T::RuntimeConfig = table
            .try_into()
            .context("could not parse blobstore runtime config")?;
        let provider = provider_type
            .make_store(runtime_config)
            .context("could not make blobstore from runtime config")?;
        Ok(Arc::new(provider))
    })
}

/// Converts from toml based runtime configuration into a [`RuntimeConfig`].
///
/// The various container types (i.e., the "type" field in the toml field) are registered with the
/// resolver using `add_store_type`. The default store for a label is registered using `add_default_store`.
#[derive(Default, Clone)]
pub struct RuntimeConfigResolver {
    /// A map of store types to a function that returns the appropriate store
    /// manager from runtime config TOML.
    store_types: HashMap<&'static str, StoreFromToml>,
}

impl RuntimeConfigResolver {
    /// Create a new RuntimeConfigResolver.
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    /// Registers a store type to the resolver.
    pub fn register_store_type<T: MakeBlobStore>(&mut self, store_type: T) -> anyhow::Result<()> {
        if self
            .store_types
            .insert(T::RUNTIME_CONFIG_TYPE, store_from_toml_fn(store_type))
            .is_some()
        {
            anyhow::bail!("duplicate blob store type {:?}", T::RUNTIME_CONFIG_TYPE);
        }
        Ok(())
    }

    /// Resolves a toml table into a runtime config.
    pub fn resolve(&self, table: Option<&impl GetTomlValue>) -> anyhow::Result<RuntimeConfig> {
        let runtime_config = self.resolve_from_toml(table)?.unwrap_or_default();
        Ok(runtime_config)
    }

    fn resolve_from_toml(
        &self,
        table: Option<&impl GetTomlValue>,
    ) -> anyhow::Result<Option<RuntimeConfig>> {
        let Some(table) = table.and_then(|t| t.get("blob_store")) else {
            return Ok(None);
        };
        let table: HashMap<String, ContainerConfig> = table.clone().try_into()?;

        let mut runtime_config = RuntimeConfig::default();
        for (label, config) in table {
            let store_manager = self
                .container_manager_from_config(config)
                .with_context(|| format!("could not configure blob store with label '{label}'"))?;
            runtime_config.add_container_manager(label.clone(), store_manager);
        }

        Ok(Some(runtime_config))
    }

    /// Given a [`ContainerConfig`], returns a container manager.
    ///
    /// Errors if there is no [`MakeBlobStore`] registered for the container config's type
    /// or if the container manager cannot be created from the config.
    fn container_manager_from_config(
        &self,
        config: ContainerConfig,
    ) -> anyhow::Result<Arc<dyn ContainerManager>> {
        let config_type = config.type_.as_str();
        let maker = self.store_types.get(config_type).with_context(|| {
            format!("the store type '{config_type}' was not registered with the config resolver")
        })?;
        maker(config.config)
    }
}

#[derive(Deserialize, Clone)]
pub struct ContainerConfig {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(flatten)]
    pub config: toml::Table,
}

impl ContainerConfig {
    pub fn new<T>(type_: String, config: T) -> anyhow::Result<Self>
    where
        T: Serialize,
    {
        Ok(Self {
            type_,
            config: toml::value::Table::try_from(config)?,
        })
    }
}
