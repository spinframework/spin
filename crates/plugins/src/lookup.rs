use crate::{error::*, git::GitSource, manifest::PluginManifest, store::manifest_file_name};
use semver::Version;
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use url::Url;

// Name of directory that contains the cloned centralized Spin plugins
// repository
const PLUGINS_REPO_LOCAL_DIRECTORY: &str = ".spin-plugins";

// Name of directory containing the installed manifests
pub(crate) const PLUGINS_REPO_MANIFESTS_DIRECTORY: &str = "manifests";

pub(crate) const SPIN_PLUGINS_REPO: &str = "https://github.com/spinframework/spin-plugins/";

/// Looks up plugin manifests in centralized spin plugin repository.
pub struct PluginLookup {
    pub name: String,
    pub version: Option<Version>,
}

impl PluginLookup {
    pub fn new(name: &str, version: Option<Version>) -> Self {
        Self {
            name: name.to_lowercase(),
            version,
        }
    }

    pub async fn resolve_manifest(
        &self,
        plugins_dir: &Path,
        skip_compatibility_check: bool,
        spin_version: &str,
    ) -> PluginLookupResult<PluginManifest> {
        let exact = self.resolve_manifest_exact(plugins_dir).await?;
        if skip_compatibility_check
            || self.version.is_some()
            || exact.is_compatible_spin_version(spin_version)
        {
            return Ok(exact);
        }

        let store = crate::store::PluginStore::new(plugins_dir.to_owned());

        // TODO: This is very similar to some logic in the badger module - look for consolidation opportunities.
        let manifests = store.catalogue_manifests()?;
        let relevant_manifests = manifests.into_iter().filter(|m| m.name() == self.name);
        let compatible_manifests = relevant_manifests
            .filter(|m| m.has_compatible_package() && m.is_compatible_spin_version(spin_version));
        let highest_compatible_manifest =
            compatible_manifests.max_by_key(|m| m.try_version().unwrap_or_else(|_| null_version()));

        Ok(highest_compatible_manifest.unwrap_or(exact))
    }

    pub async fn resolve_manifest_exact(
        &self,
        plugins_dir: &Path,
    ) -> PluginLookupResult<PluginManifest> {
        let url = plugins_repo_url()?;
        tracing::info!("Pulling manifest for plugin {} from {url}", self.name);
        fetch_plugins_repo(&url, plugins_dir, false)
            .await
            .map_err(|e| {
                Error::ConnectionFailed(ConnectionFailedError::new(url.to_string(), e.to_string()))
            })?;

        self.resolve_manifest_exact_from_good_repo(plugins_dir)
    }

    // This is split from resolve_manifest_exact because it may recurse (once) and that makes
    // Rust async sad. So we move the potential recursion to a sync helper.
    #[allow(clippy::let_and_return)]
    pub fn resolve_manifest_exact_from_good_repo(
        &self,
        plugins_dir: &Path,
    ) -> PluginLookupResult<PluginManifest> {
        let expected_path = spin_plugins_repo_manifest_path(&self.name, &self.version, plugins_dir);

        let not_found = |e: std::io::Error| {
            Err(Error::NotFound(NotFoundError::new(
                Some(self.name.clone()),
                expected_path.display().to_string(),
                e.to_string(),
            )))
        };

        let manifest = match File::open(&expected_path) {
            Ok(file) => serde_json::from_reader(file).map_err(|e| {
                Error::InvalidManifest(InvalidManifestError::new(
                    Some(self.name.clone()),
                    expected_path.display().to_string(),
                    e.to_string(),
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.version.is_some() => {
                // If a user has asked for a version by number, and the path doesn't exist,
                // it _might_ be because it's the latest version. This checks for that case.
                let latest = Self::new(&self.name, None);
                match latest.resolve_manifest_exact_from_good_repo(plugins_dir) {
                    Ok(manifest) if manifest.try_version().ok() == self.version => Ok(manifest),
                    _ => not_found(e),
                }
            }
            Err(e) => not_found(e),
        };

        manifest
    }
}

pub fn plugins_repo_url() -> Result<Url, url::ParseError> {
    Url::parse(SPIN_PLUGINS_REPO)
}

#[cfg(not(test))]
fn accept_as_repo(git_root: &Path) -> bool {
    git_root.join(".git").exists()
}

#[cfg(test)]
fn accept_as_repo(git_root: &Path) -> bool {
    git_root.join(".git").exists() || git_root.join("_spin_test_dot_git").exists()
}

pub async fn fetch_plugins_repo(
    repo_url: &Url,
    plugins_dir: &Path,
    update: bool,
) -> anyhow::Result<()> {
    let git_root = plugin_manifests_repo_path(plugins_dir);
    let git_source = GitSource::new(repo_url, None, &git_root);
    if accept_as_repo(&git_root) {
        if update {
            git_source.pull().await?;
        }
    } else {
        git_source.clone_repo().await?;
    }
    Ok(())
}

fn plugin_manifests_repo_path(plugins_dir: &Path) -> PathBuf {
    plugins_dir.join(PLUGINS_REPO_LOCAL_DIRECTORY)
}

// Given a name and option version, outputs expected file name for the plugin.
fn manifest_file_name_version(plugin_name: &str, version: &Option<semver::Version>) -> String {
    match version {
        Some(v) => format!("{plugin_name}@{v}.json"),
        None => manifest_file_name(plugin_name),
    }
}

/// Get expected path to the manifest of a plugin with a given name
/// and version within the spin-plugins repository
fn spin_plugins_repo_manifest_path(
    plugin_name: &str,
    plugin_version: &Option<Version>,
    plugins_dir: &Path,
) -> PathBuf {
    spin_plugins_repo_manifest_dir(plugins_dir)
        .join(plugin_name)
        .join(manifest_file_name_version(plugin_name, plugin_version))
}

pub fn spin_plugins_repo_manifest_dir(plugins_dir: &Path) -> PathBuf {
    plugins_dir
        .join(PLUGINS_REPO_LOCAL_DIRECTORY)
        .join(PLUGINS_REPO_MANIFESTS_DIRECTORY)
}

fn null_version() -> semver::Version {
    semver::Version::new(0, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_NAME: &str = "some-spin-ver-some-not";
    const TESTS_STORE_DIR: &str = "tests";

    fn tests_store_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TESTS_STORE_DIR)
    }

    #[tokio::test]
    async fn if_no_version_given_and_latest_is_compatible_then_latest() -> PluginLookupResult<()> {
        let lookup = PluginLookup::new(TEST_NAME, None);
        let resolved = lookup
            .resolve_manifest(&tests_store_dir(), false, "99.0.0")
            .await?;
        assert_eq!("99.0.1", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_no_version_given_and_latest_is_not_compatible_then_highest_compatible(
    ) -> PluginLookupResult<()> {
        // NOTE: The setup assumes you are NOT running Windows on aarch64, so as to check 98.1.0 is not
        // offered. If that assumption fails then this test will fail with actual version being 98.1.0.
        // (We use this combination because the OS and architecture enums don't allow for fake operating systems!)
        let lookup = PluginLookup::new(TEST_NAME, None);
        let resolved = lookup
            .resolve_manifest(&tests_store_dir(), false, "98.0.0")
            .await?;
        assert_eq!("98.0.0", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_version_given_it_gets_used_regardless() -> PluginLookupResult<()> {
        let lookup = PluginLookup::new(TEST_NAME, Some(semver::Version::parse("99.0.0").unwrap()));
        let resolved = lookup
            .resolve_manifest(&tests_store_dir(), false, "98.0.0")
            .await?;
        assert_eq!("99.0.0", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_latest_version_given_it_gets_used_regardless() -> PluginLookupResult<()> {
        let lookup = PluginLookup::new(TEST_NAME, Some(semver::Version::parse("99.0.1").unwrap()));
        let resolved = lookup
            .resolve_manifest(&tests_store_dir(), false, "98.0.0")
            .await?;
        assert_eq!("99.0.1", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_no_version_given_but_skip_compat_then_highest() -> PluginLookupResult<()> {
        let lookup = PluginLookup::new(TEST_NAME, None);
        let resolved = lookup
            .resolve_manifest(&tests_store_dir(), true, "98.0.0")
            .await?;
        assert_eq!("99.0.1", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_non_existent_version_given_then_error() -> PluginLookupResult<()> {
        let lookup = PluginLookup::new(TEST_NAME, Some(semver::Version::parse("177.7.7").unwrap()));
        lookup
            .resolve_manifest(&tests_store_dir(), true, "99.0.0")
            .await
            .expect_err("Should have errored because plugin v177.7.7 does not exist");
        Ok(())
    }
}
