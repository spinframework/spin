use crate::{
    SPIN_INTERNAL_COMMANDS,
    error::*,
    lookup::PluginRef,
    manifest::{PluginManifest, PluginPackage, warn_unsupported_version},
    store::PluginStore,
};

use anyhow::{Context, Result, anyhow, bail};
use path_absolutize::Absolutize;
use reqwest::{Client, header::HeaderMap};
use serde::Serialize;
use spin_common::sha256;
use std::{
    cmp::Ordering,
    fs::{self, File},
    io::{Cursor, copy},
    path::{Path, PathBuf},
};
use tempfile::{TempDir, tempdir};
use url::Url;

// Url scheme prefix of a plugin that is installed from a local source
const URL_FILE_SCHEME: &str = "file";

/// Location of manifest of the plugin to be installed.
pub enum ManifestLocation {
    /// Plugin manifest can be copied from a local path.
    Local(PathBuf),
    /// Plugin manifest should be pulled from a specific address.
    Remote(Url),
    /// Plugin manifest lives in the centralized plugins repository
    PluginsRepository(PluginRef),
}

impl ManifestLocation {
    pub(crate) fn to_install_record(&self) -> RawInstallRecord {
        match self {
            Self::Local(path) => {
                // Plugin commands don't absolutise on the way in, so do it now.
                use std::borrow::Cow;
                let abs = path
                    .absolutize()
                    .unwrap_or(Cow::Borrowed(path))
                    .to_path_buf();
                RawInstallRecord::Local { file: abs }
            }
            Self::Remote(url) => RawInstallRecord::Remote {
                url: url.to_owned(),
            },
            Self::PluginsRepository(_) => RawInstallRecord::PluginsRepository,
        }
    }
}

#[derive(Serialize)]
#[serde(rename = "snake_case", tag = "source")]
pub(crate) enum RawInstallRecord {
    PluginsRepository,
    Remote { url: Url },
    Local { file: PathBuf },
}

/// The entry point for plugin functionality. Use this to list, install, and remove
/// plugins, and to locate plugin binaries for execution.
///
/// PluginManager also provides access to the catalogue of available manifests via
/// the `catalogue()` function. It also provides for synchronised catalogue updates.
pub struct PluginManager {
    store: PluginStore,
}

impl PluginManager {
    /// Creates a `PluginManager` with the default install location.
    pub fn try_default() -> anyhow::Result<Self> {
        let store = PluginStore::try_default()?;
        Ok(Self { store })
    }

    /// Installs the Spin plugin with the given manifest If installing a plugin from the centralized
    /// Spin plugins repository, it fetches the latest contents of the repository and searches for
    /// the appropriately named and versioned plugin manifest. Parses the plugin manifest to get the
    /// appropriate source for the machine OS and architecture. Verifies the checksum of the source,
    /// unpacks and installs it into the plugins directory.
    /// Returns name of plugin that was successfully installed.
    pub async fn install(
        &self,
        plugin_manifest: &PluginManifest,
        plugin_package: &PluginPackage,
        source: &ManifestLocation,
        auth_header_value: &Option<String>,
    ) -> Result<String> {
        let target = plugin_package.url.to_owned();
        let target_url = Url::parse(&target)?;
        let temp_dir = tempdir()?;
        let plugin_tarball_path = match target_url.scheme() {
            URL_FILE_SCHEME => {
                let path = target_url
                    .to_file_path()
                    .map_err(|_| anyhow!("Invalid file URL: {target_url:?}"))?;
                if path.is_file() {
                    path
                } else {
                    bail!(
                        "Package path {} does not exist or is not a file",
                        path.display()
                    );
                }
            }
            _ => {
                download_plugin(
                    &plugin_manifest.name(),
                    &temp_dir,
                    &target,
                    auth_header_value,
                )
                .await?
            }
        };
        verify_checksum(&plugin_tarball_path, &plugin_package.sha256)?;

        self.store
            .untar_plugin(&plugin_tarball_path, &plugin_manifest.name())
            .with_context(|| format!("Failed to untar {}", plugin_tarball_path.display()))?;

        // Save manifest to installed plugins directory
        self.store.add_manifest(plugin_manifest)?;
        self.write_install_record(&plugin_manifest.name(), source);

        Ok(plugin_manifest.name())
    }

    /// Installs the latest (default) version of the given plugin from the
    /// catalogue, checking for compatbility against the given Spin version
    /// (unfortunately we can't infer this because this is a crate not a command).
    ///
    /// This is roughly equivalent to `spin plugins install <name>` with no options.
    pub async fn install_latest(&self, name: &str, spin_version: &str) -> anyhow::Result<String> {
        let manifest_location = ManifestLocation::PluginsRepository(PluginRef {
            name: name.to_string(),
            version: None,
        });
        let plugin_manifest = self
            .get_manifest(&manifest_location, false, spin_version, &None)
            .await?;
        let plugin_package = plugin_manifest
            .get_package()
            .context("Plugin does not contain a compatible package")?;
        self.install(&plugin_manifest, plugin_package, &manifest_location, &None)
            .await
    }

    /// Uninstalls a plugin with a given name, removing it and it's manifest from the local plugins
    /// directory.
    /// Returns true if plugin was successfully uninstalled and false if plugin did not exist.
    pub fn uninstall(&self, plugin_name: &str) -> Result<bool> {
        let plugin_store = &self.store;
        let manifest_file = plugin_store.installed_manifest_path(plugin_name);
        let exists = manifest_file.exists();
        if exists {
            // Remove the manifest and the plugin installation directory
            fs::remove_file(manifest_file)?;
            fs::remove_dir_all(plugin_store.plugin_subdirectory_path(plugin_name))?;
        }
        Ok(exists)
    }

    /// Checks manifest to see if the plugin is compatible with the running version of Spin, does
    /// not have a conflicting name with Spin internal commands, and is not a downgrade of a
    /// currently installed plugin.
    pub fn check_manifest(
        &self,
        plugin_manifest: &PluginManifest,
        spin_version: &str,
        override_compatibility_check: bool,
        allow_downgrades: bool,
    ) -> Result<InstallAction> {
        // Disallow installing plugins with the same name as spin internal subcommands
        if SPIN_INTERNAL_COMMANDS
            .iter()
            .any(|&s| s == plugin_manifest.name())
        {
            bail!(
                "Can't install a plugin with the same name ('{}') as an internal command",
                plugin_manifest.name()
            );
        }

        // Disallow reinstalling identical plugins and downgrading unless permitted.
        if let Ok(installed) = self.get_installed_manifest(&plugin_manifest.name()) {
            if &installed == plugin_manifest {
                return Ok(InstallAction::NoAction {
                    name: plugin_manifest.name(),
                    version: installed.version,
                });
            } else if installed.compare_versions(plugin_manifest) == Some(Ordering::Greater)
                && !allow_downgrades
            {
                bail!(
                    "Newer version {} of plugin '{}' is already installed. To downgrade to version {}, run `spin plugins upgrade` with the `--downgrade` flag.",
                    installed.version,
                    plugin_manifest.name(),
                    plugin_manifest.version,
                );
            }
        }

        warn_unsupported_version(plugin_manifest, spin_version, override_compatibility_check)?;

        Ok(InstallAction::Continue)
    }

    /// Fetches a manifest from a local, remote, or repository location and returned the parsed
    /// PluginManifest object.
    pub async fn get_manifest(
        &self,
        manifest_location: &ManifestLocation,
        skip_compatibility_check: bool,
        spin_version: &str,
        auth_header_value: &Option<String>,
    ) -> PluginLookupResult<PluginManifest> {
        let plugin_manifest = match manifest_location {
            ManifestLocation::Remote(url) => {
                tracing::info!("Pulling manifest for plugin from {url}");
                let client = Client::new();
                client
                    .get(url.as_ref())
                    .headers(request_headers(auth_header_value)?)
                    .send()
                    .await
                    .map_err(|e| {
                        Error::ConnectionFailed(ConnectionFailedError::new(
                            url.as_str().to_string(),
                            e.to_string(),
                        ))
                    })?
                    .error_for_status()
                    .map_err(|e| {
                        Error::ConnectionFailed(ConnectionFailedError::new(
                            url.as_str().to_string(),
                            e.to_string(),
                        ))
                    })?
                    .json::<PluginManifest>()
                    .await
                    .map_err(|e| {
                        Error::InvalidManifest(InvalidManifestError::new(
                            None,
                            url.as_str().to_string(),
                            e.to_string(),
                        ))
                    })?
            }
            ManifestLocation::Local(path) => {
                tracing::info!("Pulling manifest for plugin from {}", path.display());
                let file = File::open(path).map_err(|e| {
                    Error::NotFound(NotFoundError::new(
                        None,
                        path.display().to_string(),
                        e.to_string(),
                    ))
                })?;
                serde_json::from_reader(file).map_err(|e| {
                    Error::InvalidManifest(InvalidManifestError::new(
                        None,
                        path.display().to_string(),
                        e.to_string(),
                    ))
                })?
            }
            ManifestLocation::PluginsRepository(lookup) => {
                lookup
                    .resolve_manifest(&self.catalogue(), skip_compatibility_check, spin_version)
                    .await?
            }
        };
        Ok(plugin_manifest)
    }

    /// Returns the PluginManifest for an installed plugin with a given name.
    /// Looks up and parses the JSON plugin manifest file into object form.
    pub fn get_installed_manifest(&self, plugin_name: &str) -> PluginLookupResult<PluginManifest> {
        let manifest_path = self.store.installed_manifest_path(plugin_name);
        tracing::info!("Reading plugin manifest from {}", manifest_path.display());
        let manifest_file = File::open(manifest_path.clone()).map_err(|e| {
            Error::NotFound(NotFoundError::new(
                Some(plugin_name.to_string()),
                manifest_path.display().to_string(),
                e.to_string(),
            ))
        })?;
        let manifest = serde_json::from_reader(manifest_file).map_err(|e| {
            Error::InvalidManifest(InvalidManifestError::new(
                Some(plugin_name.to_string()),
                manifest_path.display().to_string(),
                e.to_string(),
            ))
        })?;
        Ok(manifest)
    }

    pub fn is_empty(&self) -> bool {
        let manifests_dir = self.store.installed_manifests_directory();
        if !manifests_dir.exists() {
            return true;
        }
        let Ok(mut rd) = manifests_dir.read_dir() else {
            return true;
        };
        rd.next().is_none()
    }

    pub fn installed_plugins(&self) -> anyhow::Result<Vec<PluginManifest>> {
        let manifests_dir = self.store.installed_manifests_directory();
        let manifest_paths = crate::util::json_files_in(&manifests_dir);
        let manifests = manifest_paths
            .iter()
            .filter_map(|path| crate::util::try_read_manifest_from(path))
            .collect();
        Ok(manifests)
    }

    pub async fn installed_plugins_latest_versions(
        &self,
        skip_compatibility_check: bool,
        spin_version: &str,
        auth_header_value: &Option<String>,
    ) -> anyhow::Result<Vec<(PluginManifest, ManifestLocation)>> {
        let mut plugins = vec![];

        let manifests_dir = self.store.installed_manifests_directory();

        for plugin in std::fs::read_dir(manifests_dir)? {
            let path = plugin?.path();
            let name = path
                .file_stem()
                .ok_or_else(|| anyhow!("No stem for path {}", path.display()))?
                .to_str()
                .ok_or_else(|| anyhow!("Cannot convert path {} stem to str", path.display()))?
                .to_string();
            let manifest_location =
                ManifestLocation::PluginsRepository(PluginRef::new(&name, None));
            let manifest = match self
                .get_manifest(
                    &manifest_location,
                    skip_compatibility_check,
                    spin_version,
                    auth_header_value,
                )
                .await
            {
                Err(Error::NotFound(e)) => {
                    tracing::info!("Could not upgrade plugin '{name}': {e:?}");
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(m) => m,
            };

            plugins.push((manifest, manifest_location));
        }

        Ok(plugins)
    }

    pub fn is_installed(&self, plugin_name: &str) -> bool {
        self.installed_plugins()
            .unwrap_or_default()
            .iter()
            .any(|m| m.name() == plugin_name)
    }

    pub fn is_installed_exact(&self, manifest: &PluginManifest) -> bool {
        match self.get_installed_manifest(&manifest.name()) {
            Ok(m) => m.eq(manifest),
            Err(_) => false,
        }
    }

    pub async fn update(&self) -> Result<()> {
        let mut locker = self.update_lock().await;
        let guard = locker.lock_updates();
        if guard.denied() {
            anyhow::bail!("Another plugin update operation is already in progress");
        }

        let url = crate::catalogue::plugins_repo_url()?;
        self.catalogue().fetch_from_remote(&url).await?;
        Ok(())
    }

    async fn update_lock(&self) -> PluginManagerUpdateLock {
        let lock = self.update_lock_impl().await;
        PluginManagerUpdateLock::from(lock)
    }

    async fn update_lock_impl(&self) -> anyhow::Result<fd_lock::RwLock<tokio::fs::File>> {
        let plugins_dir = self.store.get_plugins_directory();
        tokio::fs::create_dir_all(plugins_dir).await?;
        let file = tokio::fs::File::create(plugins_dir.join(".updatelock")).await?;
        let locker = fd_lock::RwLock::new(file);
        Ok(locker)
    }

    pub fn catalogue(&self) -> crate::Catalogue {
        self.store.catalogue()
    }

    pub fn installed_binary_path(&self, plugin_name: &str) -> PathBuf {
        self.store.installed_binary_path(plugin_name)
    }

    fn write_install_record(&self, plugin_name: &str, source: &ManifestLocation) {
        let install_record_path = self.store.installation_record_file(plugin_name);

        // A failure here shouldn't fail the install
        let install_record = source.to_install_record();
        if let Ok(record_text) = serde_json::to_string_pretty(&install_record) {
            _ = std::fs::write(install_record_path, record_text);
        }
    }
}

// We permit the "locking failed" state rather than erroring so that we don't prevent the user
// from doing updates just because something is amiss in the lock system. (This is basically
// falling back to the previous, never-lock, behaviour.) Put another way, we prevent updates
// only if we can _positively confirm_ that another update is in progress.
pub enum PluginManagerUpdateLock {
    Lock(fd_lock::RwLock<tokio::fs::File>),
    Failed,
}

impl From<anyhow::Result<fd_lock::RwLock<tokio::fs::File>>> for PluginManagerUpdateLock {
    fn from(value: anyhow::Result<fd_lock::RwLock<tokio::fs::File>>) -> Self {
        match value {
            Ok(lock) => Self::Lock(lock),
            Err(_) => Self::Failed,
        }
    }
}

impl PluginManagerUpdateLock {
    pub fn lock_updates(&mut self) -> PluginManagerUpdateGuard<'_> {
        match self {
            Self::Lock(lock) => match lock.try_write() {
                Ok(guard) => PluginManagerUpdateGuard::Acquired(guard),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    PluginManagerUpdateGuard::Denied
                }
                _ => PluginManagerUpdateGuard::Failed,
            },
            Self::Failed => PluginManagerUpdateGuard::Failed,
        }
    }
}

#[must_use]
pub enum PluginManagerUpdateGuard<'lock> {
    Acquired(fd_lock::RwLockWriteGuard<'lock, tokio::fs::File>),
    Denied,
    Failed, // See comment on PluginManagerUpdateLock
}

impl PluginManagerUpdateGuard<'_> {
    pub fn denied(&self) -> bool {
        matches!(self, Self::Denied)
    }
}

/// The action required to install a plugin to the desired version.
pub enum InstallAction {
    /// The installation needs to continue.
    Continue,
    /// No further action is required. This occurs when the plugin is already at the desired version.
    NoAction { name: String, version: String },
}

async fn download_plugin(
    name: &str,
    temp_dir: &TempDir,
    target_url: &str,
    auth_header_value: &Option<String>,
) -> Result<PathBuf> {
    tracing::trace!("Trying to get tar file for plugin '{name}' from {target_url}");
    let client = Client::new();
    let plugin_bin = client
        .get(target_url)
        .headers(request_headers(auth_header_value)?)
        .send()
        .await?;
    if !plugin_bin.status().is_success() {
        match plugin_bin.status() {
            reqwest::StatusCode::NOT_FOUND => bail!(
                "The download URL specified in the plugin manifest was not found ({target_url} returned HTTP error 404). Please contact the plugin author."
            ),
            _ => bail!(
                "HTTP error {} when downloading plugin from {target_url}",
                plugin_bin.status()
            ),
        }
    }

    let mut content = Cursor::new(plugin_bin.bytes().await?);
    let dir = temp_dir.path();
    let mut plugin_file = dir.join(name);
    plugin_file.set_extension("tar.gz");
    let mut temp_file = File::create(&plugin_file)?;
    copy(&mut content, &mut temp_file)?;
    Ok(plugin_file)
}

fn verify_checksum(plugin_file: &Path, expected_sha256: &str) -> Result<()> {
    let actual_sha256 = sha256::hex_digest_from_file(plugin_file)
        .with_context(|| format!("Cannot get digest for {}", plugin_file.display()))?;
    if actual_sha256 == expected_sha256 {
        tracing::info!("Package checksum verified successfully");
        Ok(())
    } else {
        Err(anyhow!("Checksum did not match, aborting installation."))
    }
}

/// Get the request headers for a call to the plugin API
///
/// If set, this will include the user provided authorization header.
fn request_headers(auth_header_value: &Option<String>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if let Some(auth_value) = auth_header_value {
        headers.insert(reqwest::header::AUTHORIZATION, auth_value.parse()?);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn good_error_when_tarball_404s() -> anyhow::Result<()> {
        let temp_dir = tempdir()?;
        let store = PluginStore::new(temp_dir.path());
        let manager = PluginManager { store };

        let bad_manifest: PluginManifest = serde_json::from_str(include_str!(
            "../tests/nonexistent-url/nonexistent-url.json"
        ))?;

        let install_result = manager
            .install(
                &bad_manifest,
                &bad_manifest.packages[0],
                &ManifestLocation::Local(PathBuf::from(
                    "../tests/nonexistent-url/nonexistent-url.json",
                )),
                &None,
            )
            .await;

        let err = format!("{:#}", install_result.unwrap_err());
        assert!(
            err.contains("not found"),
            "Expected error to contain 'not found' but was '{err}'"
        );

        Ok(())
    }
}
