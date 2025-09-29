use std::fmt::Debug;

use async_trait::async_trait;

use crate::Key;

/// A config provider.
#[async_trait]
pub trait Provider: Debug + Send + Sync {
    /// Returns the value at the given config path, if it exists.
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>>;
    fn kind(&self) -> ProviderVariableKind;
}

/// The dynamism of a Provider.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ProviderVariableKind {
    /// Variable must be declared on start
    Static,
    /// Variable can be made available at runtime
    #[default]
    Dynamic,
}
