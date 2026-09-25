use serde::{Deserialize, Serialize};
use spin_expressions::async_trait::async_trait;
use spin_factors::anyhow::{self, Context as _};
use tracing::{Level, instrument};
use vaultrs::{
    client::{VaultClient, VaultClientSettingsBuilder},
    error::ClientError,
    kv2,
};

use spin_expressions::{Key, Provider};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
/// A config Provider that uses HashiCorp Vault.
pub struct VaultVariablesProvider {
    /// The URL of the Vault server.
    url: String,
    /// The token to authenticate with.
    token: String,
    /// The mount point of the KV engine.
    mount: String,
    /// The optional prefix to use for all keys.
    #[serde(default)]
    prefix: Option<String>,
}

#[async_trait]
impl Provider for VaultVariablesProvider {
    #[instrument(name = "spin_variables.get_from_vault", level = Level::DEBUG, skip(self), err(level = Level::INFO), fields(otel.kind = "client"))]
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        let client = self.try_get_client()?;
        try_get_value(&client, &self.mount, key.as_str(), self.prefix.clone())
            .await
            .context("Failed to retrieve config from Vault")
    }
}

impl VaultVariablesProvider {
    fn try_get_client(&self) -> anyhow::Result<VaultClient> {
        VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(&self.url)
                .token(&self.token)
                .build()?,
        )
        .context("Failed to create Vault client")
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
/// A config provider that uses OpenBao
pub struct OpenBaoVariableProvider {
    /// The URL of the Vault server.
    url: String,
    /// The token to authenticate with.
    token: String,
    /// The mount point of the KV engine.
    mount: String,
    /// The optional prefix to use for all keys.
    #[serde(default)]
    prefix: Option<String>,
}

#[async_trait]
impl Provider for OpenBaoVariableProvider {
    #[instrument(name = "spin_variables.get_from_openbao", level = Level::DEBUG, skip(self), err(level = Level::INFO), fields(otel.kind = "client"))]
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        let client = self.try_get_client()?;
        try_get_value(&client, &self.mount, key.as_str(), self.prefix.clone())
            .await
            .context("Failed to retrieve config from OpenBao")
    }
}

async fn try_get_value(
    client: &VaultClient,
    mount_point: &str,
    key: &str,
    prefix: Option<String>,
) -> anyhow::Result<Option<String>> {
    let key_path = match prefix {
        Some(prefix) => format!("{}/{}", prefix, key),
        None => key.to_string(),
    };

    #[derive(Deserialize, Serialize)]
    struct Secret {
        value: String,
    }

    match kv2::read::<Secret>(client, mount_point, &key_path).await {
        Ok(secret) => Ok(Some(secret.value)),
        // Vault doesn't have this entry so pass along the chain
        Err(ClientError::APIError { code: 404, .. }) => Ok(None),
        // Other Vault error so bail rather than looking elsewhere
        Err(e) => Err(e).context("Failed to check provider for config"),
    }
}

impl OpenBaoVariableProvider {
    fn try_get_client(&self) -> anyhow::Result<VaultClient> {
        VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(&self.url)
                .token(&self.token)
                .build()?,
        )
        .context("Failed to create OpenBao client")
    }
}
