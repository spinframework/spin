const SPIN_ENV_REPO: &str = "https://github.com/spinframework/spin-environments";
const ENVS_DIR_IN_REPO: &str = "envs";

pub struct Catalogue {
    git_root: PathBuf,
    envs_root: PathBuf,
}

static CATALOGUE_UPDATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Catalogue {
    pub fn try_default() -> anyhow::Result<Self> {
        let root = dirs::cache_dir()
            .ok_or(anyhow::anyhow!("No system cache directory"))?
            .join("spin")
            .join("environments");
        Ok(Self::new(root))
    }

    fn new(git_root: PathBuf) -> Self {
        Self {
            git_root: git_root.clone(),
            envs_root: git_root.join(ENVS_DIR_IN_REPO),
        }
    }

    pub async fn update(&self) -> anyhow::Result<()> {
        // We don't want two git pulls running concurrently
        let _guard = CATALOGUE_UPDATE_LOCK.lock();

        let url = Url::parse(SPIN_ENV_REPO)?;
        let git_source = GitSource::new(&url, None, &self.git_root);
        if self.git_root.exists() {
            git_source.pull().await
        } else {
            tokio::fs::create_dir_all(&self.git_root).await?;
            git_source.clone_repo().await
        }
    }

    /// This requires `env_id` to be normalised to the `ns@version` form
    pub async fn get(&self, env_id: &str) -> anyhow::Result<Option<EnvironmentDefinition>> {
        // We add (redundant) directories to avoid having a single flat
        // namespace that becomes unmanageable.
        //
        // ENV_ROOT
        // |-- foo
        // |  |-- foo@1.2.toml
        // |  |-- foo@1.6.toml
        // |-- bar
        // |  |-- bar.toml
        let ns = sans_version(env_id);
        // TODO: I suppose we should stop people making up path injectiony kind of names
        // although I am unconvinced such a thing would get you anything you don't have already
        let path = self.envs_root.join(ns).join(format!("{env_id}.toml"));
        if !path.exists() {
            return Ok(None);
        }
        let toml_text = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("Environment '{env_id}' not found"))?;
        let env_def = toml::from_str(&toml_text)
            .with_context(|| format!("Environment '{env_id}' definition is invalid format"))?;
        Ok(Some(env_def))
    }
}

fn sans_version(id: &str) -> &str {
    match id.rsplit_once('@') {
        None => id,
        Some((stem, _)) => stem,
    }
}

// From here on this is a copy of plugins/git.rs, which itself was
// recycled from templates...

use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use url::Url;

use crate::environment::definition::EnvironmentDefinition;

const DEFAULT_BRANCH: &str = "main";

/// Enables cloning and fetching the latest of a git repository to a local
/// directory.
pub struct GitSource {
    /// Address to remote git repository.
    source_url: Url,
    /// Branch to clone/fetch.
    branch: String,
    /// Destination to clone repository into.
    git_root: PathBuf,
}

impl GitSource {
    /// Creates a new git source
    pub fn new(source_url: &Url, branch: Option<String>, git_root: impl AsRef<Path>) -> GitSource {
        Self {
            source_url: source_url.clone(),
            branch: branch.unwrap_or_else(|| DEFAULT_BRANCH.to_owned()),
            git_root: git_root.as_ref().to_owned(),
        }
    }

    /// Clones a contents of a git repository to a local directory
    pub async fn clone_repo(&self) -> Result<()> {
        let mut git = Command::new("git");
        git.args([
            "clone",
            self.source_url.as_ref(),
            "--branch",
            &self.branch,
            "--single-branch",
        ])
        .arg(&self.git_root);
        let clone_result = git.output().await.understand_git_result();
        if let Err(e) = clone_result {
            anyhow::bail!("Error cloning Git repo {}: {}", self.source_url, e)
        }
        Ok(())
    }

    /// Fetches the latest changes from the source repository
    pub async fn pull(&self) -> Result<()> {
        let mut git = Command::new("git");
        git.arg("-C").arg(&self.git_root).arg("pull");
        let pull_result = git.output().await.understand_git_result();
        if let Err(e) = pull_result {
            anyhow::bail!(
                "Error updating Git repo at {}: {}",
                self.git_root.display(),
                e
            )
        }
        Ok(())
    }
}

// TODO: the following and templates/git.rs are duplicates

pub(crate) enum GitError {
    ProgramFailed(Vec<u8>),
    ProgramNotFound,
    Other(anyhow::Error),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProgramNotFound => f.write_str("`git` command not found - is git installed?"),
            Self::Other(e) => e.fmt(f),
            Self::ProgramFailed(stderr) => match std::str::from_utf8(stderr) {
                Ok(s) => f.write_str(s),
                Err(_) => f.write_str("(cannot get error)"),
            },
        }
    }
}

pub(crate) trait UnderstandGitResult {
    fn understand_git_result(self) -> Result<Vec<u8>, GitError>;
}

impl UnderstandGitResult for Result<std::process::Output, std::io::Error> {
    fn understand_git_result(self) -> Result<Vec<u8>, GitError> {
        match self {
            Ok(output) => {
                if output.status.success() {
                    Ok(output.stdout)
                } else {
                    Err(GitError::ProgramFailed(output.stderr))
                }
            }
            Err(e) => match e.kind() {
                // TODO: consider cases like insufficient permission?
                ErrorKind::NotFound => Err(GitError::ProgramNotFound),
                _ => {
                    let err = anyhow::Error::from(e).context("Failed to run `git` command");
                    Err(GitError::Other(err))
                }
            },
        }
    }
}
