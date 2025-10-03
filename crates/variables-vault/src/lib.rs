use serde::{Deserialize, Serialize};
use spin_expressions::async_trait::async_trait;
use spin_factors::anyhow::{self, Context as _};
use tracing::{instrument, Level};
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
        let client = VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(&self.url)
                .token(&self.token)
                .build()?,
        )?;
        let path = match &self.prefix {
            Some(prefix) => format!("{}/{}", prefix, key.as_str()),
            None => key.as_str().to_string(),
        };

        #[derive(Deserialize, Serialize)]
        struct Secret {
            value: String,
        }
        match kv2::read::<Secret>(&client, &self.mount, &path).await {
            Ok(secret) => Ok(Some(secret.value)),
            // Vault doesn't have this entry so pass along the chain
            Err(ClientError::APIError { code: 404, .. }) => Ok(None),
            // Other Vault error so bail rather than looking elsewhere
            Err(e) => Err(e).context("Failed to check Vault for config"),
        }
    }
}
