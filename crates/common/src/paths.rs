//! Resolves a file path to a manifest file

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

use crate::ui::quoted_path;

/// The name given to the default manifest file.
pub const DEFAULT_MANIFEST_FILE: &str = "spin.toml";

/// Attempts to find a manifest. If a path is provided, that path is resolved
/// using `resolve_manifest_file_path`; otherwise, a directory search is carried out
/// using `search_upwards_for_manifest`. If we had to search, and a manifest is found,
/// a (non-zero) usize is returned indicating how far above the current directory it
/// was found. (A usize of 0 indicates that the manifest was provided or found
/// in the current directory.) This can be used to notify the user that a
/// non-default manifest is being used.
pub fn find_manifest_file_path(
    provided_path: Option<impl AsRef<Path>>,
) -> Result<(PathBuf, usize)> {
    match provided_path {
        Some(provided_path) => resolve_manifest_file_path(provided_path).map(|p| (p, 0)),
        None => search_upwards_for_manifest()
            .ok_or_else(|| anyhow!("\"{}\" not found", DEFAULT_MANIFEST_FILE)),
    }
}

/// Resolves a manifest path provided by a user, which may be a file or
/// directory, to a path to a manifest file.
pub fn resolve_manifest_file_path(provided_path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = provided_path.as_ref();

    if path.is_file() {
        Ok(path.to_owned())
    } else if path.is_dir() {
        let file_path = path.join(DEFAULT_MANIFEST_FILE);
        if file_path.is_file() {
            Ok(file_path)
        } else {
            Err(anyhow!(
                "Directory {} does not contain a file named 'spin.toml'",
                path.display()
            ))
        }
    } else {
        let pd = path.display();
        let err = match path.try_exists() {
            Err(e) => anyhow!("Error accessing path {pd}: {e:#}"),
            Ok(false) => anyhow!("No such file or directory '{pd}'"),
            Ok(true) => anyhow!("Path {pd} is neither a file nor a directory"),
        };
        Err(err)
    }
}

/// Starting from the current directory, searches upward through
/// the directory tree for a manifest (that is, a file with the default
/// manifest name `spin.toml`). If found, the path to the manifest
/// is returned, with a usize indicating how far above the current directory it
/// was found. (A usize of 0 indicates that the manifest was provided or found
/// in the current directory.) This can be used to notify the user that a
/// non-default manifest is being used.
/// If no matching file is found, the function returns None.
///
/// The search is abandoned if it reaches the root directory, or the
/// root of a Git repository, without finding a 'spin.toml'.
pub fn search_upwards_for_manifest() -> Option<(PathBuf, usize)> {
    let candidate = PathBuf::from(DEFAULT_MANIFEST_FILE);

    if candidate.is_file() {
        return Some((candidate, 0));
    }

    for distance in 1..20 {
        let inferred_dir = PathBuf::from("../".repeat(distance));
        if !inferred_dir.is_dir() {
            return None;
        }

        let candidate = inferred_dir.join(DEFAULT_MANIFEST_FILE);
        if candidate.is_file() {
            return Some((candidate, distance));
        }

        if is_git_root(&inferred_dir) {
            return None;
        }
    }

    None
}

/// Resolves the parent directory of a path, returning an error if the path
/// has no parent. A path with a single component will return ".".
pub fn parent_dir(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    let mut parent = path
        .parent()
        .with_context(|| format!("No parent directory for path {}", quoted_path(path)))?;
    if parent == Path::new("") {
        parent = Path::new(".");
    }
    Ok(parent.into())
}

fn is_git_root(dir: &Path) -> bool {
    dir.join(".git").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_returns_parent() {
        assert_eq!(parent_dir("foo/bar").unwrap(), Path::new("foo"));
    }

    #[test]
    fn blank_parent_returns_dot() {
        assert_eq!(parent_dir("foo").unwrap(), Path::new("."));
    }

    #[test]
    fn no_parent_returns_err() {
        parent_dir("").unwrap_err();
    }
}
