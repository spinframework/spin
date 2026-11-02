//! Clone and update Git repositories, and classify `git` process failures.
//!
//! Plugin catalogues, template installation, and environment catalogues share
//! this helper.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use tokio::process::Command;
use url::Url;

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
    /// Creates a new git source.
    ///
    /// `branch` defaults to `main`.
    pub fn new(source_url: &Url, branch: Option<String>, git_root: impl AsRef<Path>) -> GitSource {
        Self {
            source_url: source_url.clone(),
            branch: branch.unwrap_or_else(|| DEFAULT_BRANCH.to_owned()),
            git_root: git_root.as_ref().to_owned(),
        }
    }

    /// Clones a contents of a git repository to a local directory.
    pub async fn clone_repo(&self) -> anyhow::Result<()> {
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

    /// Fetches the latest changes from the source repository.
    pub async fn pull(&self) -> anyhow::Result<()> {
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

/// Failure from running `git`.
pub enum GitError {
    /// `git` exited non-zero. The payload is its stderr.
    ProgramFailed(Vec<u8>),
    /// The `git` program could not be found.
    ProgramNotFound,
    /// Running `git` failed before it produced a status.
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

/// Classifies the outcome of a `git` invocation.
pub trait UnderstandGitResult {
    /// Returns stdout when `git` succeeded, or a [`GitError`] otherwise.
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

/// Reports whether `dir` is inside a Git work tree.
///
/// `git` is invoked with stdout and stderr discarded. An error from launching
/// `git` is returned; a non-zero status is `Ok(false)`.
pub async fn is_in_git_repo(dir: &Path) -> anyhow::Result<bool> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--git-dir")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let status = cmd
        .status()
        .await
        .context("checking if new app is in a git repo")?;
    Ok(status.success())
}

/// Runs `git init` in `dir`.
pub async fn init_git_repo(dir: &Path) -> Result<(), GitError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).arg("init");

    let result = cmd.output().await;
    result.understand_git_result().map(|_| ())
}
