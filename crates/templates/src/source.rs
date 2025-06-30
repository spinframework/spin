use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use tempfile::{tempdir, TempDir};
use tokio::process::Command;
use url::Url;

use crate::{directory::subdirectories, git::UnderstandGitResult};

const TEMPLATE_SOURCE_DIR: &str = "templates";
const TEMPLATE_VERSION_TAG_PREFIX: &str = "spin/templates/v";

/// A source from which to install templates.
#[derive(Debug)]
pub enum TemplateSource {
    /// Install from a Git repository at the specified URL. If a branch is
    /// specified, templates are installed from that branch or tag; otherwise,
    /// they are installed from HEAD.
    ///
    /// Templates much be in a `/templates` directory under the root of the
    /// repository.
    Git(GitTemplateSource),
    /// Install from a directory in the file system.
    ///
    /// Templates much be in a `/templates` directory under the specified
    /// root.
    File(PathBuf),
    /// Install from a remote tarball.
    ///
    /// Templates should be in a `/templates` directory under the root of the tarball.
    /// The implementation also allows for there to be a single root directory containing
    /// the `templates` directory - this makes it compatible with GitHub release tarballs.
    RemoteTar(Url),
}

/// Settings for installing templates from a Git repository.
#[derive(Debug)]
pub struct GitTemplateSource {
    /// The URL of the Git repository from which to install templates.
    url: Url,
    /// The branch or tag from which to install templates; inferred if omitted.
    branch: Option<String>,
    /// The version of the Spin client, used for branch inference.
    // We have to pass this through because vergen is only on the root bin
    spin_version: String,
}

impl TemplateSource {
    /// Creates a `TemplateSource` referring to the specified Git repository
    /// and branch.
    pub fn try_from_git(
        git_url: impl AsRef<str>,
        branch: &Option<String>,
        spin_version: &str,
    ) -> anyhow::Result<Self> {
        let url_str = git_url.as_ref();
        let url =
            Url::parse(url_str).with_context(|| format!("Failed to parse {url_str} as URL"))?;
        Ok(Self::Git(GitTemplateSource {
            url,
            branch: branch.clone(),
            spin_version: spin_version.to_owned(),
        }))
    }

    pub(crate) fn to_install_record(&self) -> Option<crate::reader::RawInstalledFrom> {
        match self {
            Self::Git(g) => Some(crate::reader::RawInstalledFrom::Git {
                git: g.url.to_string(),
            }),
            Self::File(p) => {
                // Saving a relative path would be meaningless (but should never happen)
                if p.is_absolute() {
                    Some(crate::reader::RawInstalledFrom::File {
                        dir: format!("{}", p.display()),
                    })
                } else {
                    None
                }
            }
            Self::RemoteTar(url) => Some(crate::reader::RawInstalledFrom::RemoteTar {
                url: url.to_string(),
            }),
        }
    }

    // Sorry I know this is a bit ugly
    /// For a Git source, resolves the tag to use as the source.
    /// For other sources, returns None.
    pub async fn resolved_tag(&self) -> Option<String> {
        match self {
            Self::Git(g) => version_matched_tag(g.url.as_str(), &g.spin_version).await,
            _ => None,
        }
    }
}

pub(crate) struct LocalTemplateSource {
    root: PathBuf,
    _temp_dir: Option<TempDir>,
}

impl TemplateSource {
    pub(crate) async fn get_local(&self) -> anyhow::Result<LocalTemplateSource> {
        match self {
            Self::Git(git_source) => clone_local(git_source).await,
            Self::File(path) => check_local(path).await,
            Self::RemoteTar(url) => download_untar_local(url).await,
        }
    }

    pub(crate) fn requires_copy(&self) -> bool {
        match self {
            Self::Git { .. } => true,
            Self::File(_) => false,
            Self::RemoteTar(_) => true,
        }
    }
}

impl LocalTemplateSource {
    pub async fn template_directories(&self) -> anyhow::Result<Vec<PathBuf>> {
        let templates_root = self.root.join(TEMPLATE_SOURCE_DIR);
        if templates_root.exists() {
            subdirectories(&templates_root).with_context(|| {
                format!("Failed to read contents of '{TEMPLATE_SOURCE_DIR}' directory")
            })
        } else {
            Err(anyhow!(
                "Template source {} does not contain a '{}' directory",
                self.root.display(),
                TEMPLATE_SOURCE_DIR
            ))
        }
    }
}

async fn clone_local(git_source: &GitTemplateSource) -> anyhow::Result<LocalTemplateSource> {
    let temp_dir = tempdir()?;
    let path = temp_dir.path().to_owned();

    let url_str = git_source.url.as_str();

    let actual_branch = match &git_source.branch {
        Some(b) => Some(b.clone()),
        None => version_matched_tag(url_str, &git_source.spin_version).await,
    };

    let mut git = Command::new("git");
    git.arg("clone");
    git.arg("--depth").arg("1");

    if let Some(b) = actual_branch {
        git.arg("--branch").arg(b);
    }

    git.arg(url_str).arg(&path);

    let clone_result = git.output().await.understand_git_result();
    match clone_result {
        Ok(_) => Ok(LocalTemplateSource {
            root: path,
            _temp_dir: Some(temp_dir),
        }),
        Err(e) => Err(anyhow!("Error cloning Git repo {}: {}", url_str, e)),
    }
}

async fn version_matched_tag(url: &str, spin_version: &str) -> Option<String> {
    let preferred_tag = version_preferred_tag(spin_version);

    let mut git = Command::new("git");
    git.arg("ls-remote");
    git.arg("--exit-code");
    git.arg(url);
    git.arg(&preferred_tag);

    match git.output().await.understand_git_result() {
        Ok(_) => Some(preferred_tag),
        Err(_) => None,
    }
}

fn version_preferred_tag(text: &str) -> String {
    let mm_version = match semver::Version::parse(text) {
        Ok(version) => format!("{}.{}", version.major, version.minor),
        Err(_) => text.to_owned(),
    };
    format!("{TEMPLATE_VERSION_TAG_PREFIX}{mm_version}")
}

async fn check_local(path: &Path) -> anyhow::Result<LocalTemplateSource> {
    if path.exists() {
        Ok(LocalTemplateSource {
            root: path.to_owned(),
            _temp_dir: None,
        })
    } else {
        Err(anyhow!("Path not found: {}", path.display()))
    }
}

/// Download a tarball to a temorary directory
async fn download_untar_local(url: &Url) -> anyhow::Result<LocalTemplateSource> {
    use bytes::buf::Buf;

    let temp_dir = tempdir()?;
    let path = temp_dir.path().to_owned();

    let resp = reqwest::get(url.clone())
        .await
        .with_context(|| format!("Failed to download from {url}"))?;
    let tar_content = resp
        .bytes()
        .await
        .with_context(|| format!("Failed to download from {url}"))?;

    let reader = flate2::read::GzDecoder::new(tar_content.reader());
    let mut archive = tar::Archive::new(reader);
    archive
        .unpack(&path)
        .context("Failed to unpack tar archive")?;

    let templates_root = bypass_gh_added_root(path);

    Ok(LocalTemplateSource {
        root: templates_root,
        _temp_dir: Some(temp_dir),
    })
}

/// GitHub adds a prefix directory to release tarballs (e.g. spin-v3.0.0/...).
/// We try to locate the repo root within the unpacked tarball.
fn bypass_gh_added_root(unpack_dir: PathBuf) -> PathBuf {
    // If the unpack dir directly contains a `templates` dir then we are done.
    if has_templates_dir(&unpack_dir) {
        return unpack_dir;
    }

    let Ok(dirs) = unpack_dir.read_dir() else {
        // If we can't traverse the unpack directory then return it and
        // let the top level try to make sense of it.
        return unpack_dir;
    };

    // Is there a single directory at the root?  If not, we can't be in the GitHub situation:
    // return the root of the unpacking. (The take(2) here is because we don't need to traverse
    // the full list - we only care whether there is more than one.)
    let dirs = dirs.filter_map(|de| de.ok()).take(2).collect::<Vec<_>>();
    if dirs.len() != 1 {
        return unpack_dir;
    }

    // If we get here, there is a single directory (dirs has a single element). Look in it to see if it's a plausible repo root.
    let candidate_repo_root = dirs[0].path();
    let Ok(mut candidate_repo_dirs) = candidate_repo_root.read_dir() else {
        // Again, if it all goes awry, propose the base unpack directory.
        return unpack_dir;
    };
    let has_templates_dir = candidate_repo_dirs.any(is_templates_dir);

    if has_templates_dir {
        candidate_repo_root
    } else {
        unpack_dir
    }
}

fn has_templates_dir(path: &Path) -> bool {
    let Ok(mut dirs) = path.read_dir() else {
        return false;
    };

    dirs.any(is_templates_dir)
}

fn is_templates_dir(dir_entry: Result<std::fs::DirEntry, std::io::Error>) -> bool {
    dir_entry.is_ok_and(|d| d.file_name() == TEMPLATE_SOURCE_DIR)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn preferred_tag_excludes_patch_version() {
        assert_eq!("spin/templates/v1.2", version_preferred_tag("1.2.3"));
    }

    #[test]
    fn preferred_tag_excludes_prerelease_and_build() {
        assert_eq!(
            "spin/templates/v1.2",
            version_preferred_tag("1.2.3-preview.1")
        );
        assert_eq!(
            "spin/templates/v1.2",
            version_preferred_tag("1.2.3+build.0f74628")
        );
        assert_eq!(
            "spin/templates/v1.2",
            version_preferred_tag("1.2.3-alpha+0f74628")
        );
    }

    #[test]
    fn preferred_tag_defaults_sensibly_on_bad_semver() {
        assert_eq!("spin/templates/v1.2", version_preferred_tag("1.2"));
        assert_eq!("spin/templates/v1.2.3.4", version_preferred_tag("1.2.3.4"));
        assert_eq!("spin/templates/vgarbage", version_preferred_tag("garbage"));
    }
}
