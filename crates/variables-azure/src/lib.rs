use std::sync::Arc;

use anyhow::Context as _;
use azure_core::{auth::TokenCredential, Url};
use azure_security_keyvault::SecretClient;
use serde::Deserialize;
use spin_expressions::{Key, Provider};
use spin_factors::anyhow;
use spin_world::async_trait;
use tracing::{instrument, Level};

/// Azure KeyVault runtime config literal options for authentication
///
/// Some of these fields are optional. Whether they are set determines whether
/// environmental variables will be used to resolve the information instead.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AzureKeyVaultVariablesConfig {
    pub vault_url: String,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub tenant_id: Option<String>,
    pub authority_host: Option<AzureAuthorityHost>,
}

#[derive(Debug, Copy, Clone, Deserialize, Default)]
pub enum AzureAuthorityHost {
    #[default]
    AzurePublicCloud,
    AzureChina,
    AzureGermany,
    AzureGovernment,
}

impl TryFrom<AzureKeyVaultVariablesConfig> for AzureKeyVaultAuthOptions {
    type Error = anyhow::Error;

    fn try_from(value: AzureKeyVaultVariablesConfig) -> Result<Self, Self::Error> {
        match (value.client_id, value.tenant_id, value.client_secret) {
            (Some(client_id), Some(tenant_id), Some(client_secret)) => Ok(
                AzureKeyVaultAuthOptions::RuntimeConfigValues{
                    client_id,
                    client_secret,
                    tenant_id,
                    authority_host: value.authority_host.unwrap_or_default(),
                }
            ),
            (None, None, None) => Ok(AzureKeyVaultAuthOptions::Environmental),
            _ => anyhow::bail!("The current runtime config specifies some but not all of the Azure KeyVault 'client_id', 'client_secret', and 'tenant_id' values. Provide the missing values to authenticate to Azure KeyVault with the given service principal, or remove all these values to authenticate using ambient authentication (e.g. env vars, Azure CLI, Managed Identity, Workload Identity).")
        }
    }
}

/// Azure Cosmos Key / Value enumeration for the possible authentication options
#[derive(Clone, Debug)]
pub enum AzureKeyVaultAuthOptions {
    /// Runtime Config values indicates the service principal credentials have been supplied
    RuntimeConfigValues {
        client_id: String,
        client_secret: String,
        tenant_id: String,
        authority_host: AzureAuthorityHost,
    },
    /// Environmental indicates that the environment variables of the process
    /// should be used to create the TokenCredential for the Cosmos client. This
    /// will use the Azure Rust SDK's DefaultCredentialChain to derive the
    /// TokenCredential based on what environment variables have been set.
    ///
    /// Service Principal with client secret:
    /// - `AZURE_TENANT_ID`: ID of the service principal's Azure tenant.
    /// - `AZURE_CLIENT_ID`: the service principal's client ID.
    /// - `AZURE_CLIENT_SECRET`: one of the service principal's secrets.
    ///
    /// Service Principal with certificate:
    /// - `AZURE_TENANT_ID`: ID of the service principal's Azure tenant.
    /// - `AZURE_CLIENT_ID`: the service principal's client ID.
    /// - `AZURE_CLIENT_CERTIFICATE_PATH`: path to a PEM or PKCS12 certificate
    ///   file including the private key.
    /// - `AZURE_CLIENT_CERTIFICATE_PASSWORD`: (optional) password for the
    ///   certificate file.
    ///
    /// Workload Identity (Kubernetes, injected by the Workload Identity
    /// mutating webhook):
    /// - `AZURE_TENANT_ID`: ID of the service principal's Azure tenant.
    /// - `AZURE_CLIENT_ID`: the service principal's client ID.
    /// - `AZURE_FEDERATED_TOKEN_FILE`: TokenFilePath is the path of a file
    ///   containing a Kubernetes service account token.
    ///
    /// Managed Identity (User Assigned or System Assigned identities):
    /// - `AZURE_CLIENT_ID`: (optional) if using a user assigned identity, this
    ///   will be the client ID of the identity.
    ///
    /// Azure CLI:
    /// - `AZURE_TENANT_ID`: (optional) use a specific tenant via the Azure CLI.
    ///
    /// Common across each:
    /// - `AZURE_AUTHORITY_HOST`: (optional) the host for the identity provider.
    ///   For example, for Azure public cloud the host defaults to
    ///   `"https://login.microsoftonline.com"`.
    ///
    /// See also:
    /// <https://github.com/Azure/azure-sdk-for-rust/blob/main/sdk/identity/README.md>
    Environmental,
}

/// A provider that fetches variables from Azure Key Vault.
#[derive(Debug)]
pub struct AzureKeyVaultProvider {
    secret_client: SecretClient,
}

impl AzureKeyVaultProvider {
    pub fn create(
        vault_url: impl Into<String>,
        auth_options: AzureKeyVaultAuthOptions,
    ) -> anyhow::Result<Self> {
        let http_client = azure_core::new_http_client();
        let token_credential = match auth_options {
            AzureKeyVaultAuthOptions::RuntimeConfigValues {
                client_id,
                client_secret,
                tenant_id,
                authority_host,
            } => {
                let credential = azure_identity::ClientSecretCredential::new(
                    http_client,
                    authority_host.into(),
                    tenant_id,
                    client_id,
                    client_secret,
                );
                Arc::new(credential) as Arc<dyn TokenCredential>
            }
            AzureKeyVaultAuthOptions::Environmental => azure_identity::create_default_credential()?,
        };

        Ok(Self {
            secret_client: SecretClient::new(&vault_url.into(), token_credential)?,
        })
    }
}

#[async_trait]
impl Provider for AzureKeyVaultProvider {
    #[instrument(name = "spin_variables.get_from_azure_key_vault", level = Level::DEBUG, skip(self), err(level = Level::INFO), fields(otel.kind = "client"))]
    async fn get(&self, key: &Key) -> anyhow::Result<Option<String>> {
        let secret = self
            .secret_client
            .get(key.as_str())
            .await
            .context("Failed to read variable from Azure Key Vault")?;
        Ok(Some(secret.value))
    }
}

impl From<AzureAuthorityHost> for Url {
    fn from(value: AzureAuthorityHost) -> Self {
        let url = match value {
            AzureAuthorityHost::AzureChina => "https://login.chinacloudapi.cn/",
            AzureAuthorityHost::AzureGovernment => "https://login.microsoftonline.us/",
            AzureAuthorityHost::AzureGermany => "https://login.microsoftonline.de/",
            AzureAuthorityHost::AzurePublicCloud => "https://login.microsoftonline.com/",
        };
        Url::parse(url).unwrap()
    }
}
