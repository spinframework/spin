use anyhow::Result;
use azure_core::credentials::TokenCredential;
use std::sync::Arc;

/// Azure Cosmos Key / Value runtime config literal options for authentication
#[derive(Clone, Debug)]
pub struct KeyValueAzureCosmosRuntimeConfigOptions {
    pub(crate) key: String,
}

impl KeyValueAzureCosmosRuntimeConfigOptions {
    pub fn new(key: String) -> Self {
        Self { key }
    }
}

/// Azure Cosmos Key / Value enumeration for the possible authentication options
#[derive(Clone, Debug)]
pub enum KeyValueAzureCosmosAuthOptions {
    /// Runtime Config values indicates the account and key have been specified directly
    RuntimeConfigValues(KeyValueAzureCosmosRuntimeConfigOptions),
    /// An Azure AD token credential, used when the runtime config omits `key`.
    ///
    /// The specific credential is chosen by the operator via the `auth_type`
    /// runtime-config field (defaulting to developer tools for local
    /// development). There is deliberately no fallback *between* credential
    /// types: `azure_identity` 1.0 removed `EnvironmentCredential` /
    /// `DefaultAzureCredential` because silently trying a different identity
    /// after one fails is a security footgun (Azure/azure-sdk-for-rust#2283).
    /// This mirrors their recommended "specific credential" pattern.
    AadCredential(AzureCredentialKind),
}

/// The specific Azure AD credential to use when authenticating to Cosmos
/// without an account key.
///
/// Each variant maps to exactly one `azure_identity` credential; the operator
/// names the one matching their deployment via the `auth_type` runtime-config
/// field. Modeled on `azure_identity`'s `specific_credential.rs` example.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AzureCredentialKind {
    /// Developer tools: Azure CLI (`az login`), then Azure Developer CLI
    /// (`azd auth login`). Intended for local development; the default when
    /// `auth_type` is omitted.
    #[default]
    DeveloperTools,
    /// Managed identity (Azure VM, App Service, or AKS with managed identity).
    ///
    /// `client_id` optionally selects a *user-assigned* managed identity by its
    /// client ID; when `None`, the SDK uses the *system-assigned* identity.
    ManagedIdentity { client_id: Option<String> },
    /// Workload identity (AKS federated token).
    WorkloadIdentity,
    /// Service principal authenticated with a client secret. Reads
    /// `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, and `AZURE_CLIENT_SECRET` from the
    /// environment — the same variables the legacy SDK's `EnvironmentCredential`
    /// used, so existing deployments keep working without config changes.
    ServicePrincipal,
}

impl AzureCredentialKind {
    /// Parses the `auth_type` runtime-config value into a credential kind.
    ///
    /// `None` (the field omitted) defaults to [`AzureCredentialKind::DeveloperTools`],
    /// intended for local development. An unrecognized value is an error rather
    /// than a silent fallback.
    ///
    /// `client_id` selects a user-assigned managed identity by its client ID. It
    /// only applies to `managed_identity`; with any other `auth_type` it is
    /// ignored.
    pub fn from_auth_type(auth_type: Option<&str>, client_id: Option<String>) -> Result<Self> {
        // Case-insensitive, but the value must be one of the canonical
        // snake_case names: a non-canonical form (e.g. a space or hyphen
        // separator) is rejected rather than silently normalized, so the
        // accepted set matches exactly what the docs and the error below list.
        // `client_id` is consumed only by the `managed_identity` arm; for any
        // other auth type it is simply dropped.
        match auth_type.map(|s| s.to_lowercase()).as_deref() {
            None => Ok(Self::default()),
            Some("developer_tools") => Ok(Self::DeveloperTools),
            Some("managed_identity") => Ok(Self::ManagedIdentity { client_id }),
            Some("workload_identity") => Ok(Self::WorkloadIdentity),
            Some("service_principal") => Ok(Self::ServicePrincipal),
            Some(other) => anyhow::bail!(
                "unknown Azure Cosmos `auth_type` {other:?}; expected one of \
                 \"managed_identity\", \"workload_identity\", \"service_principal\", \
                 or \"developer_tools\" (or set `key` for account-key auth)"
            ),
        }
    }

    /// Constructs the corresponding `azure_identity` token credential.
    ///
    /// This runs the credential's own setup (which may fail — e.g. if the
    /// environment for workload identity or service principal is absent), so it
    /// is called lazily when the Cosmos client is first built.
    pub(crate) fn credential(&self) -> azure_core::Result<Arc<dyn TokenCredential>> {
        match self {
            Self::DeveloperTools => Ok(azure_identity::DeveloperToolsCredential::new(None)?),
            Self::ManagedIdentity { client_id } => {
                // Pass options only when a user-assigned client ID was given;
                // `None` keeps the SDK default of the system-assigned identity.
                let options =
                    client_id
                        .as_ref()
                        .map(|id| azure_identity::ManagedIdentityCredentialOptions {
                            user_assigned_id: Some(azure_identity::UserAssignedId::ClientId(
                                id.clone(),
                            )),
                            ..Default::default()
                        });
                Ok(azure_identity::ManagedIdentityCredential::new(options)?)
            }
            Self::WorkloadIdentity => Ok(azure_identity::WorkloadIdentityCredential::new(None)?),
            Self::ServicePrincipal => {
                // azure_identity 1.0 removed the env-driven `EnvironmentCredential`,
                // so read the same variables it used and pass them to
                // `ClientSecretCredential` explicitly. A missing variable surfaces
                // here (lazily, when the client is first built) as a clear error.
                let tenant_id = service_principal_env("AZURE_TENANT_ID")?;
                let client_id = service_principal_env("AZURE_CLIENT_ID")?;
                let secret = service_principal_env("AZURE_CLIENT_SECRET")?;
                Ok(azure_identity::ClientSecretCredential::new(
                    &tenant_id,
                    client_id,
                    secret.into(),
                    None,
                )?)
            }
        }
    }
}

/// Reads a required Service Principal environment variable, mapping a missing
/// value to a clear credential error.
fn service_principal_env(name: &str) -> azure_core::Result<String> {
    std::env::var(name).map_err(|_| {
        azure_core::Error::with_message(
            azure_core::error::ErrorKind::Credential,
            format!(
                "Azure Cosmos `service_principal` auth requires the `{name}` \
                 environment variable to be set"
            ),
        )
    })
}
