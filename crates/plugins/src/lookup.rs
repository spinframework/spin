use crate::{Catalogue, catalogue::plugins_repo_url, error::*, manifest::PluginManifest};
use semver::Version;
use std::fs::File;

/// Looks up plugin manifests in centralized spin plugin repository.
pub struct PluginRef {
    pub name: String,
    pub version: Option<Version>,
}

impl PluginRef {
    pub fn new(name: &str, version: Option<Version>) -> Self {
        Self {
            name: name.to_lowercase(),
            version,
        }
    }

    /// This looks up this reference in the current snapshot, but if the reference
    /// is missing or incompatible with the given version of Spin and the current OS
    /// and processor environment, then it tries to find a fallback version
    /// in the snapshot that *will* work. This is the "eager to please" resolver.
    pub(crate) async fn resolve_manifest(
        &self,
        catalogue: &Catalogue,
        skip_compatibility_check: bool,
        spin_version: &str,
    ) -> PluginLookupResult<PluginManifest> {
        let exact = self.resolve_manifest_exact(catalogue).await?;
        if skip_compatibility_check
            || self.version.is_some()
            || exact.is_compatible_spin_version(spin_version)
        {
            return Ok(exact);
        }

        // TODO: This is very similar to some logic in the badger module - look for consolidation opportunities.
        let manifests = catalogue.manifests()?;
        let relevant_manifests = manifests.into_iter().filter(|m| m.name() == self.name);
        let compatible_manifests = relevant_manifests
            .filter(|m| m.has_compatible_package() && m.is_compatible_spin_version(spin_version));
        let highest_compatible_manifest =
            compatible_manifests.max_by_key(|m| m.try_version().unwrap_or_else(|_| null_version()));

        Ok(highest_compatible_manifest.unwrap_or(exact))
    }

    /// This looks up this **exact** reference in the current snapshot. The snapshot
    /// will not be refreshed, but it may be initialised if it does not yet exist.
    /// Compatibility is not considered; no alternative versions are considered.
    pub(crate) async fn resolve_manifest_exact(
        &self,
        catalogue: &Catalogue,
    ) -> PluginLookupResult<PluginManifest> {
        let url = plugins_repo_url()?;
        tracing::info!("Pulling manifest for plugin {} from {url}", self.name);
        catalogue.ensure_inited(&url).await.map_err(|e| {
            Error::ConnectionFailed(ConnectionFailedError::new(url.to_string(), e.to_string()))
        })?;

        self.resolve_manifest_exact_from_good_repo(catalogue)
    }

    // This is split from resolve_manifest_exact because it may recurse (once) and that makes
    // Rust async sad. So we move the potential recursion to a sync helper.
    #[allow(clippy::let_and_return)]
    fn resolve_manifest_exact_from_good_repo(
        &self,
        catalogue: &Catalogue,
    ) -> PluginLookupResult<PluginManifest> {
        let expected_path = catalogue.manifest_path(&self.name, &self.version);

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
                match latest.resolve_manifest_exact_from_good_repo(catalogue) {
                    Ok(manifest) if manifest.try_version().ok() == self.version => Ok(manifest),
                    _ => not_found(e),
                }
            }
            Err(e) => not_found(e),
        };

        manifest
    }
}

fn null_version() -> semver::Version {
    semver::Version::new(0, 0, 0)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::store::PluginStore;

    const TEST_NAME: &str = "some-spin-ver-some-not";
    const TESTS_STORE_DIR: &str = "tests";

    fn tests_store_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(TESTS_STORE_DIR)
    }

    fn tests_store() -> Catalogue {
        PluginStore::new(tests_store_dir()).catalogue()
    }

    #[tokio::test]
    async fn if_no_version_given_and_latest_is_compatible_then_latest() -> PluginLookupResult<()> {
        let lookup = PluginRef::new(TEST_NAME, None);
        let resolved = lookup
            .resolve_manifest(&tests_store(), false, "99.0.0")
            .await?;
        assert_eq!("99.0.1", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_no_version_given_and_latest_is_not_compatible_then_highest_compatible()
    -> PluginLookupResult<()> {
        // NOTE: The setup assumes you are NOT running Windows on aarch64, so as to check 98.1.0 is not
        // offered. If that assumption fails then this test will fail with actual version being 98.1.0.
        // (We use this combination because the OS and architecture enums don't allow for fake operating systems!)
        let lookup = PluginRef::new(TEST_NAME, None);
        let resolved = lookup
            .resolve_manifest(&tests_store(), false, "98.0.0")
            .await?;
        assert_eq!("98.0.0", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_version_given_it_gets_used_regardless() -> PluginLookupResult<()> {
        let lookup = PluginRef::new(TEST_NAME, Some(semver::Version::parse("99.0.0").unwrap()));
        let resolved = lookup
            .resolve_manifest(&tests_store(), false, "98.0.0")
            .await?;
        assert_eq!("99.0.0", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_latest_version_given_it_gets_used_regardless() -> PluginLookupResult<()> {
        let lookup = PluginRef::new(TEST_NAME, Some(semver::Version::parse("99.0.1").unwrap()));
        let resolved = lookup
            .resolve_manifest(&tests_store(), false, "98.0.0")
            .await?;
        assert_eq!("99.0.1", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_no_version_given_but_skip_compat_then_highest() -> PluginLookupResult<()> {
        let lookup = PluginRef::new(TEST_NAME, None);
        let resolved = lookup
            .resolve_manifest(&tests_store(), true, "98.0.0")
            .await?;
        assert_eq!("99.0.1", resolved.version);
        Ok(())
    }

    #[tokio::test]
    async fn if_non_existent_version_given_then_error() -> PluginLookupResult<()> {
        let lookup = PluginRef::new(TEST_NAME, Some(semver::Version::parse("177.7.7").unwrap()));
        lookup
            .resolve_manifest(&tests_store(), true, "99.0.0")
            .await
            .expect_err("Should have errored because plugin v177.7.7 does not exist");
        Ok(())
    }
}
