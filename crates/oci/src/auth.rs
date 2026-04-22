use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use oci_distribution::secrets::RegistryAuth;
use serde::{Deserialize, Serialize};
use spin_common::ui::quoted_path;

#[derive(Serialize, Deserialize)]
pub struct AuthConfig {
    /// Map between registry server and base64 encoded username:password credential set.
    pub auths: HashMap<String, String>,
}

impl AuthConfig {
    /// Load the authentication configuration from the default location
    /// ($XDG_CONFIG_HOME/fermyon/registry-auth.json).
    pub async fn load_default() -> Result<Self> {
        // TODO: add a way to override this path.
        match Self::load(&Self::default_path()?).await {
            Ok(s) => Ok(s),
            Err(_) => Ok(Self {
                auths: HashMap::new(),
            }),
        }
    }

    /// Save the authentication configuration to the default location
    /// ($XDG_CONFIG_HOME/fermyon/registry-auth.json).
    pub async fn save_default(&self) -> Result<()> {
        self.save(&Self::default_path()?).await
    }

    /// Insert the new credentials into the auths file, with the server as the key and base64
    /// encoded username:password as the value.
    pub fn insert(
        &mut self,
        server: impl AsRef<str>,
        username: impl AsRef<str>,
        password: impl AsRef<str>,
    ) -> Result<()> {
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", username.as_ref(), password.as_ref()),
        );
        self.auths.insert(server.as_ref().to_string(), encoded);

        Ok(())
    }

    fn default_path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("Cannot find configuration directory")?
            .join("fermyon")
            .join("registry-auth.json"))
    }

    /// Get the registry authentication for a given registry from the default location.
    pub async fn get_auth_from_default(server: impl AsRef<str>) -> Result<RegistryAuth> {
        let auths = Self::load_default().await?;
        let encoded = match auths.auths.get(server.as_ref()) {
            Some(e) => e,
            None => bail!(format!("no credentials stored for {}", server.as_ref())),
        };

        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)?;
        let decoded = std::str::from_utf8(&bytes)?;
        let parts: Vec<&str> = decoded.splitn(2, ':').collect();

        tracing::trace!("Decoded registry credentials from the Spin configuration.");
        Ok(RegistryAuth::Basic(
            parts
                .first()
                .context("expected username as first element of the decoded auth")?
                .to_string(),
            parts
                .get(1)
                .context("expected secret as second element of the decoded auth")?
                .to_string(),
        ))
    }

    async fn load(p: &Path) -> Result<Self> {
        let contents = tokio::fs::read_to_string(&p).await?;
        serde_json::from_str(&contents)
            .with_context(|| format!("cannot load authentication file {}", quoted_path(p)))
    }

    async fn save(&self, p: &Path) -> Result<()> {
        if let Some(parent_dir) = p.parent() {
            tokio::fs::create_dir_all(parent_dir)
                .await
                .with_context(|| format!("Failed to create config dir {}", parent_dir.display()))?;
        }
        tokio::fs::write(&p, &serde_json::to_vec_pretty(&self)?)
            .await
            .with_context(|| format!("cannot save authentication file {}", quoted_path(p)))
    }
}
