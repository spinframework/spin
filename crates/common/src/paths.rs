//! Resolves a file path to a manifest file

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

use crate::ui::quoted_path;

/// The name given to the default manifest file.
pub const DEFAULT_MANIFEST_FILE: &str = "spin.toml";

/// The name given to the default standalone component manifest file.
pub const DEFAULT_COMPONENT_MANIFEST_FILE: &str = "component.toml";

/// The manifest filenames considered by commands that can operate on either an
/// application manifest or a standalone component manifest (such as `spin build`,
/// and, in future, the registry commands). Listed in order of preference: an
/// application manifest (`spin.toml`), then a component manifest (`component.toml`).
const APP_OR_COMPONENT_MANIFEST_CANDIDATES: &[&str] =
    &[DEFAULT_MANIFEST_FILE, DEFAULT_COMPONENT_MANIFEST_FILE];

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
    find_manifest_file_path_from(provided_path, &[DEFAULT_MANIFEST_FILE])
}

/// Like [`find_manifest_file_path`], but also considers a standalone component
/// manifest (`component.toml`) when resolving a directory or searching the
/// directory tree. This is used by commands that can operate on either an
/// application manifest or a component manifest (such as `spin build`). An
/// application manifest is preferred if both are present.
pub fn find_app_or_component_manifest_file_path(
    provided_path: Option<impl AsRef<Path>>,
) -> Result<(PathBuf, usize)> {
    find_manifest_file_path_from(provided_path, APP_OR_COMPONENT_MANIFEST_CANDIDATES)
}

fn find_manifest_file_path_from(
    provided_path: Option<impl AsRef<Path>>,
    candidates: &[&str],
) -> Result<(PathBuf, usize)> {
    match provided_path {
        Some(provided_path) => {
            resolve_manifest_file_path_from(provided_path, candidates).map(|p| (p, 0))
        }
        None => search_upwards_for_manifest_from(candidates)
            .ok_or_else(|| anyhow!("\"{}\" not found", candidates.join("\" or \""))),
    }
}

/// Resolves a manifest path provided by a user, which may be a file or
/// directory, to a path to a manifest file.
pub fn resolve_manifest_file_path(provided_path: impl AsRef<Path>) -> Result<PathBuf> {
    resolve_manifest_file_path_from(provided_path, &[DEFAULT_MANIFEST_FILE])
}

fn resolve_manifest_file_path_from(
    provided_path: impl AsRef<Path>,
    candidates: &[&str],
) -> Result<PathBuf> {
    let path = provided_path.as_ref();

    if path.is_file() {
        Ok(path.to_owned())
    } else if path.is_dir() {
        for candidate in candidates {
            let file_path = path.join(candidate);
            if file_path.is_file() {
                return Ok(file_path);
            }
        }
        Err(anyhow!(
            "Directory {} does not contain a file named {}",
            path.display(),
            candidates
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(" or ")
        ))
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
    search_upwards_for_manifest_from(&[DEFAULT_MANIFEST_FILE])
}

fn search_upwards_for_manifest_from(candidates: &[&str]) -> Option<(PathBuf, usize)> {
    for candidate in candidates {
        let candidate = PathBuf::from(candidate);
        if candidate.is_file() {
            return Some((candidate, 0));
        }
    }

    for distance in 1..20 {
        let inferred_dir = PathBuf::from("../".repeat(distance));
        if !inferred_dir.is_dir() {
            return None;
        }

        for candidate in candidates {
            let candidate = inferred_dir.join(candidate);
            if candidate.is_file() {
                return Some((candidate, distance));
            }
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
