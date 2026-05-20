use crate::{git::GitSource, manifest::PluginManifest};
use anyhow::Context;
use semver::Version;
use std::path::{Path, PathBuf};
use url::Url;

const SPIN_PLUGINS_REPO: &str = "https://github.com/spinframework/spin-plugins/";

pub(crate) fn plugins_repo_url() -> Result<Url, url::ParseError> {
    Url::parse(SPIN_PLUGINS_REPO)
}

/// The local clone of the spin-plugins repo.
pub struct Catalogue {
    git_root: PathBuf,
    manifests_root: PathBuf,
}

// Name of directory containing the installed manifests
const LOCAL_CATALOGUE_MANIFESTS_DIRECTORY: &str = "manifests";

impl Catalogue {
    pub fn new(git_root: PathBuf) -> Self {
        let manifests_root = git_root.join(LOCAL_CATALOGUE_MANIFESTS_DIRECTORY);
        Self {
            git_root,
            manifests_root,
        }
    }

    pub fn manifests(&self) -> anyhow::Result<Vec<PluginManifest>> {
        // Structure:
        // CATALOGUE_DIR (spin/plugins/.spin-plugins/manifests)
        // |- foo
        // |  |- foo@0.1.2.json
        // |  |- foo@1.2.3.json
        // |  |- foo.json
        // |- bar
        //    |- bar.json
        let catalogue_manifests_dir = &self.manifests_root;

        // Catalogue directory doesn't exist so likely nothing has been installed.
        if !catalogue_manifests_dir.exists() {
            return Ok(Vec::new());
        }

        let plugin_dirs = catalogue_manifests_dir
            .read_dir()
            .with_context(|| format!("reading manifest catalogue at {catalogue_manifests_dir:?}"))?
            .filter_map(|d| d.ok())
            .map(|d| d.path())
            .filter(|p| p.is_dir());
        let manifest_paths = plugin_dirs.flat_map(|path| crate::util::json_files_in(&path));
        let manifests: Vec<_> = manifest_paths
            .filter_map(|path| crate::util::try_read_manifest_from(&path))
            .collect();
        Ok(manifests)
    }

    /// Get expected path to the manifest of a plugin with a given name
    /// and version within the spin-plugins repository
    pub(crate) fn manifest_path(
        &self,
        plugin_name: &str,
        plugin_version: &Option<Version>,
    ) -> PathBuf {
        self.manifests_root
            .join(plugin_name)
            .join(crate::util::manifest_file_name_version(
                plugin_name,
                plugin_version,
            ))
    }

    /// Clones or pulls the spin-plugins repo as required. THIS IS NOT SYNCHRONISED
    /// and should be used only if you know nothing else is updating the working
    /// copy at the same time: generally, prefer `PluginManager::update()` which
    /// checks for contention.
    pub(crate) async fn fetch_from_remote(&self, repo_url: &Url) -> anyhow::Result<()> {
        let git_root = &self.git_root;
        let git_source = GitSource::new(repo_url, None, git_root);
        if accept_as_repo(git_root) {
            git_source.pull().await?;
        } else {
            git_source.clone_repo().await?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_inited(&self, repo_url: &Url) -> anyhow::Result<()> {
        let git_root = &self.git_root;
        let git_source = GitSource::new(repo_url, None, git_root);
        if !accept_as_repo(git_root) {
            git_source.clone_repo().await?;
        }
        Ok(())
    }
}

#[cfg(not(test))]
fn accept_as_repo(git_root: &Path) -> bool {
    git_root.join(".git").exists()
}

#[cfg(test)]
fn accept_as_repo(git_root: &Path) -> bool {
    git_root.join(".git").exists() || git_root.join("_spin_test_dot_git").exists()
}
