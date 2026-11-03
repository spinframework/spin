//! Prepared application directories.
//!
//! A prepared directory holds everything a Spin application from a registry
//! needs to run, laid out ahead of time so that `spin up --from-prepared` can
//! run it in place, without copying its files or accessing a registry:
//!
//! ```text
//! <dir>/
//!   spin.prepared.lock      the locked app, with sources relative to <dir>
//!   wasm/<sha256 hex>.wasm  component, dependency and trigger dependency Wasm
//!   assets/<component id>/  each component's files, as the guest sees them
//! ```
//!
//! In the lock:
//!
//! - every Wasm source is exactly `wasm/<hex>.wasm`, where `<hex>` is the
//!   SHA-256 of the file, and keeps its `sha256:<hex>` digest;
//! - every file has an entry of its own, whose source is exactly
//!   `assets/<component id>/<path>` and whose digest is the SHA-256 of the
//!   file;
//! - sources are relative paths with `/` separators, not URLs;
//! - a component without files has no `assets/<component id>` directory;
//! - the `origin` metadata names the artifact pinned to its digest, and
//!   `prepared_layout_version` is the version of this layout.
//!
//! The directory contains no symbolic links.
//!
//! This is layout version 1. A Spin refuses a directory whose layout
//! version it does not read, so a change to the layout comes with a new
//! version.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};
use oci_client::Reference;
use oci_client::manifest::OciImageManifest;
use spin_common::sha256::hex_digest_from_file;
use spin_common::ui::quoted_path;
use spin_loader::cache::Cache;
use spin_locked_app::locked::{ContentPath, ContentRef, LockedApp, LockedComponent};
use spin_serde::KebabId;

use crate::Client;
use crate::loader::{
    ASSETS_DIR, cached_lockfile_path, content_digest, is_safe_to_join, read_locked_app,
    wasm_content_refs_mut, write_component_files,
};

/// The name of the locked app in a prepared directory.
pub const PREPARED_LOCK_FILE: &str = "spin.prepared.lock";

/// The locked app metadata key that holds the version of the prepared layout.
const LAYOUT_VERSION_KEY: &str = "prepared_layout_version";

/// The version of the prepared layout that this module writes and reads.
const LAYOUT_VERSION: u64 = 1;

/// The directory of a prepared app that holds its Wasm files.
const WASM_DIR: &str = "wasm";

/// Writes the Spin application that a pull of `reference` resolved to
/// `manifest` and `digest` (see [`Client::pull_with_digest`]) to `dir` as a
/// prepared application.
///
/// `dir` must not exist or must be an empty directory, which is replaced (so
/// it cannot be a mount point). The application is written to a temporary
/// directory next to `dir` and moved into place at the end, so that a
/// failure leaves nothing half-written.
pub async fn prepare(
    client: &Client,
    reference: &str,
    manifest: &OciImageManifest,
    digest: &str,
    dir: &Path,
) -> Result<()> {
    let lockfile_path = cached_lockfile_path(client, reference, digest).await?;
    let origin = pinned_reference(reference, digest)?;
    let Some(app) = read_locked_app(manifest, &lockfile_path, &origin).await? else {
        bail!("{reference:?} is a Wasm package, not a Spin application, and cannot be prepared");
    };
    write_prepared(app, &client.cache, dir).await
}

/// Checks that `dir` names a directory that is missing or empty, and is not a
/// symbolic link, so that an application can be prepared in it.
pub fn check_target(dir: &Path) -> Result<()> {
    ensure!(
        dir.file_name().is_some(),
        "{} does not name a directory to prepare the app in",
        quoted_path(dir)
    );
    let metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("cannot use {}", quoted_path(dir))),
    };
    ensure!(
        !metadata.file_type().is_symlink(),
        "{} is a symbolic link",
        quoted_path(dir)
    );
    ensure!(metadata.is_dir(), "{} is not a directory", quoted_path(dir));
    let mut entries =
        std::fs::read_dir(dir).with_context(|| format!("cannot read {}", quoted_path(dir)))?;
    ensure!(
        entries.next().is_none(),
        "{} is not empty",
        quoted_path(dir)
    );
    Ok(())
}

/// Writes a locked app read from the registry cache to `dir` as a prepared
/// application.
async fn write_prepared(mut app: LockedApp, cache: &Cache, dir: &Path) -> Result<()> {
    check_target(dir)?;
    validate_lock(&app)?;

    let staging = create_staging_dir(dir)?;
    for component in &mut app.components {
        let id = component.id.clone();
        prepare_component(component, cache, staging.path())
            .await
            .with_context(|| format!("failed to prepare component {id:?}"))?;
    }
    write_lock(&mut app, staging.path()).await?;

    // On failure, dropping `staging` removes it.
    move_into_place(staging.path(), dir).await?;
    // `staging` is now `dir`, which must be kept.
    let _ = staging.keep();
    Ok(())
}

/// The artifact reference pinned to its digest, keeping the tag if any.
fn pinned_reference(reference: &str, digest: &str) -> Result<String> {
    let parsed = Reference::try_from(reference).context("invalid reference")?;
    let (registry, repository) = (parsed.registry().to_owned(), parsed.repository().to_owned());
    let pinned = match parsed.tag() {
        Some(tag) => {
            Reference::with_tag_and_digest(registry, repository, tag.to_owned(), digest.to_owned())
        }
        None => Reference::with_digest(registry, repository, digest.to_owned()),
    };
    Ok(pinned.whole())
}

/// Creates the directory to write the prepared app in. It is next to `dir`,
/// so that it can be renamed to `dir`.
fn create_staging_dir(dir: &Path) -> Result<tempfile::TempDir> {
    let parent = match dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", quoted_path(parent)))?;
    tempfile::Builder::new()
        .prefix(".spin-prepare-")
        .tempdir_in(parent)
        .with_context(|| format!("failed to create a directory in {}", quoted_path(parent)))
}

/// Copies a component's Wasm and files to the prepared directory `dir`, and
/// points their sources there.
async fn prepare_component(
    component: &mut LockedComponent,
    cache: &Cache,
    dir: &Path,
) -> Result<()> {
    copy_wasm(component, cache, dir).await?;
    copy_files(component, cache, dir).await
}

/// Copies the component's Wasm, and that of its dependencies and trigger
/// dependencies, from the cache to `wasm/<hex>.wasm`. Wasm used more than once
/// is copied once.
///
/// The cache is not checked when it is read, so each copy is checked against
/// its digest: the lock must record what the directory holds.
async fn copy_wasm(component: &mut LockedComponent, cache: &Cache, dir: &Path) -> Result<()> {
    for content in wasm_content_refs_mut(component) {
        let digest = content_digest(content)?.to_owned();
        let source = wasm_source(&digest)?;
        let dest = dir.join(&source);
        if !dest.exists() {
            let cached = cache.wasm_file(&digest)?;
            copy_file(&cached, &dest).await?;
            ensure!(
                matches_digest(&dest, &digest)?,
                "{} in the cache does not match its digest {digest}",
                quoted_path(&cached)
            );
        }
        *content = prepared_content(source, digest);
    }
    Ok(())
}

/// Writes the component's files to `assets/<component id>/`, as the guest
/// sees them, and records the source and digest of each.
///
/// As for Wasm, each file with a digest is checked against it once written.
/// Inline files may have no digest, and then theirs is computed.
async fn copy_files(component: &mut LockedComponent, cache: &Cache, dir: &Path) -> Result<()> {
    write_component_files(
        &component.id,
        &component.files,
        cache,
        &dir.join(ASSETS_DIR),
    )
    .await?;
    for file in &mut component.files {
        let source = asset_source(&component.id, &file.path);
        let written = dir.join(&source);
        let digest = match &file.content.digest {
            Some(digest) => {
                ensure!(
                    matches_digest(&written, digest)?,
                    "file {} does not match its digest {digest}",
                    quoted_path(&file.path)
                );
                digest.clone()
            }
            None => file_digest(&written)?,
        };
        file.content = prepared_content(source, digest);
    }
    Ok(())
}

/// The content reference to a file of the prepared directory.
fn prepared_content(source: String, digest: String) -> ContentRef {
    ContentRef {
        source: Some(source),
        inline: None,
        digest: Some(digest),
    }
}

async fn copy_file(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", quoted_path(parent)))?;
    }
    tokio::fs::copy(src, dest)
        .await
        .with_context(|| format!("failed to copy {}", quoted_path(src)))?;
    Ok(())
}

/// Writes the lock, with the layout version, to the prepared directory `dir`.
async fn write_lock(app: &mut LockedApp, dir: &Path) -> Result<()> {
    app.metadata
        .insert(LAYOUT_VERSION_KEY.to_owned(), LAYOUT_VERSION.into());
    let path = dir.join(PREPARED_LOCK_FILE);
    tokio::fs::write(&path, app.to_json()?)
        .await
        .with_context(|| format!("failed to write {}", quoted_path(&path)))
}

/// Moves the staged directory to `dir`, replacing it if it is an empty
/// directory.
async fn move_into_place(staged: &Path, dir: &Path) -> Result<()> {
    // Elsewhere, rename replaces an empty directory atomically.
    #[cfg(windows)]
    if dir.exists() {
        tokio::fs::remove_dir(dir)
            .await
            .with_context(|| format!("failed to replace {}", quoted_path(dir)))?;
    }
    tokio::fs::rename(staged, dir).await.with_context(|| {
        format!(
            "failed to move the prepared app to {} (it must be missing or an empty directory, and not a mount point)",
            quoted_path(dir)
        )
    })
}

/// Checks the parts of a locked app that the prepared layout depends on:
/// each component id names a directory, each file path names a file in it,
/// and trigger dependencies belong to components.
fn validate_lock(app: &LockedApp) -> Result<()> {
    ensure!(
        app.triggers
            .iter()
            .all(|trigger| trigger.trigger_dependencies.is_empty()),
        "the app has trigger dependencies outside its components, which a prepared app cannot have"
    );
    check_ids_are_unique(app)?;
    for component in &app.components {
        let id = &component.id;
        KebabId::try_from(id.clone())
            .map_err(|reason| anyhow!("invalid component id {id:?}: {reason}"))?;
        for file in &component.files {
            check_file_path(&file.path).with_context(|| format!("component {id:?}"))?;
        }
        check_files_are_listed_once(component).with_context(|| format!("component {id:?}"))?;
    }
    Ok(())
}

fn check_ids_are_unique(app: &LockedApp) -> Result<()> {
    let mut seen = HashSet::new();
    for component in &app.components {
        if !seen.insert(component.id.as_str()) {
            bail!("component id {:?} is used more than once", component.id);
        }
    }
    Ok(())
}

fn check_files_are_listed_once(component: &LockedComponent) -> Result<()> {
    let mut seen = HashSet::new();
    for file in &component.files {
        if !seen.insert(file.path.as_path()) {
            bail!("file {} is listed more than once", quoted_path(&file.path));
        }
    }
    Ok(())
}

/// Checks that a file path names a file below the component's directory: a
/// non-empty relative path of plain names.
fn check_file_path(path: &Path) -> Result<()> {
    // Locks record file paths with `/` separators. A backslash is a separator
    // on Windows only, so it would name different files on different hosts.
    ensure!(
        path.components().next().is_some()
            && is_safe_to_join(path)
            && !path.to_string_lossy().contains('\\'),
        "invalid file path {}",
        quoted_path(path)
    );
    Ok(())
}

/// The source a prepared lock records for a Wasm file: `wasm/<hex>.wasm`,
/// named after its SHA-256 digest.
fn wasm_source(digest: &str) -> Result<String> {
    let hex = digest
        .strip_prefix("sha256:")
        .filter(|hex| {
            hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
        .with_context(|| format!("unsupported Wasm digest {digest:?}: expected sha256:<hex>"))?;
    Ok(format!("{WASM_DIR}/{hex}.wasm"))
}

/// The source a prepared lock records for a component file:
/// `assets/<component id>/<path>`, with `/` separators. `path` must have
/// passed [`check_file_path`].
fn asset_source(component_id: &str, path: &Path) -> String {
    let names: Vec<_> = path
        .components()
        .map(|name| name.as_os_str().to_string_lossy())
        .collect();
    format!("{ASSETS_DIR}/{component_id}/{}", names.join("/"))
}

/// The `sha256:<hex>` digest of a file.
fn file_digest(path: &Path) -> Result<String> {
    let hex = hex_digest_from_file(path)
        .with_context(|| format!("failed to read {}", quoted_path(path)))?;
    Ok(format!("sha256:{hex}"))
}

/// Whether the file matches its `sha256:<hex>` digest.
fn matches_digest(path: &Path, digest: &str) -> Result<bool> {
    Ok(file_digest(path)? == digest)
}

/// Reads the prepared application in `dir`, checks the directory against its
/// lock, and returns a locked app that Spin can run from `dir` in place.
///
/// `dir` must contain no symbolic links. Each Wasm and file source in the
/// lock must be the path the layout gives it, and each component's
/// `assets/<component id>` directory, which is what the guest sees, must hold
/// exactly its listed files. With `verify_digests`, every file is also
/// checked against its digest. This reads the directory, and blocks while it
/// does.
///
/// These checks hold when this function runs: Spin opens the files again
/// later, so the caller must ensure that `dir` does not change while the app
/// runs (for example, a read-only mount). The digests come from the lock in
/// `dir`, so they detect a corrupted or partly changed directory, not a
/// different application: trust a prepared directory as you would the code
/// in it. Hard links are not detected.
///
/// In the returned app, Wasm sources are `file:` URLs into `dir`, and each
/// component's files become one mount of `assets/<component id>` at `/`, as
/// for apps loaded from a registry. Nothing is written to `dir`.
pub fn load(dir: &Path, verify_digests: bool) -> Result<LockedApp> {
    let mut prepared = PreparedDir::scan(dir, verify_digests)?;
    let mut app = prepared.read_lock()?;
    validate_lock(&app)?;
    for component in &mut app.components {
        let id = component.id.clone();
        prepared
            .load_component(component)
            .with_context(|| format!("component {id:?} does not match the prepared directory"))?;
    }
    Ok(app)
}

/// A prepared directory being loaded.
struct PreparedDir {
    root: PathBuf,
    /// The regular files in the directory, relative to it.
    files: HashSet<PathBuf>,
    /// The directories in the directory, relative to it.
    dirs: HashSet<PathBuf>,
    verify_digests: bool,
    /// The Wasm files already checked against their digests.
    checked_wasm: HashSet<String>,
}

impl PreparedDir {
    /// Lists the prepared directory `dir`, which must contain only regular
    /// files and directories. A symbolic link could point outside `dir`, and
    /// the guest would see what it points to.
    fn scan(dir: &Path, verify_digests: bool) -> Result<Self> {
        let root = std::fs::canonicalize(dir)
            .with_context(|| format!("cannot use {}", quoted_path(dir)))?;
        let mut files = HashSet::new();
        let mut dirs = HashSet::new();
        for entry in walkdir::WalkDir::new(&root).min_depth(1) {
            let entry = entry.with_context(|| format!("cannot read {}", quoted_path(&root)))?;
            let path = entry.path().strip_prefix(&root)?.to_owned();
            let file_type = entry.file_type();
            if file_type.is_file() {
                files.insert(path);
            } else if file_type.is_dir() {
                dirs.insert(path);
            } else if file_type.is_symlink() {
                bail!(
                    "{} is a symbolic link, which a prepared directory cannot contain",
                    quoted_path(entry.path())
                );
            } else {
                bail!(
                    "{} is not a regular file or a directory",
                    quoted_path(entry.path())
                );
            }
        }
        Ok(Self {
            root,
            files,
            dirs,
            verify_digests,
            checked_wasm: HashSet::new(),
        })
    }

    /// Reads the lock and checks its layout version.
    fn read_lock(&self) -> Result<LockedApp> {
        ensure!(
            self.has_file(PREPARED_LOCK_FILE),
            "{} is not a prepared application: it has no {PREPARED_LOCK_FILE}",
            quoted_path(&self.root)
        );
        let path = self.root.join(PREPARED_LOCK_FILE);
        let lock = std::fs::read(&path)
            .with_context(|| format!("failed to read {}", quoted_path(&path)))?;
        let app = LockedApp::from_json(&lock)
            .with_context(|| format!("failed to decode {}", quoted_path(&path)))?;
        let version = app.metadata.get(LAYOUT_VERSION_KEY);
        ensure!(
            version.and_then(|v| v.as_u64()) == Some(LAYOUT_VERSION),
            "{} has prepared layout version {}, but this Spin reads version {LAYOUT_VERSION}",
            quoted_path(&path),
            version.map_or("(none)".to_owned(), |v| v.to_string())
        );
        Ok(app)
    }

    /// Checks a component against the directory, and points its sources
    /// into it.
    fn load_component(&mut self, component: &mut LockedComponent) -> Result<()> {
        self.load_wasm(component)?;
        self.load_files(component)
    }

    /// Checks that the component's Wasm files are where the layout puts
    /// them, and points their sources at them. Wasm used more than once is
    /// checked against its digest once.
    fn load_wasm(&mut self, component: &mut LockedComponent) -> Result<()> {
        for content in wasm_content_refs_mut(component) {
            let digest = content_digest(content)?.to_owned();
            let source = wasm_source(&digest)?;
            check_source(content, &source)?;
            ensure!(
                self.has_file(&source),
                "{} is missing",
                quoted_path(&source)
            );
            let path = self.root.join(&source);
            if self.verify_digests && self.checked_wasm.insert(source) {
                check_digest(&path, &digest)?;
            }
            *content = ContentRef {
                source: Some(file_url(&path)?),
                ..Default::default()
            };
        }
        Ok(())
    }

    /// Checks that the component's directory holds exactly its listed files,
    /// and replaces them with one mount of that directory, as the guest sees
    /// it.
    fn load_files(&self, component: &mut LockedComponent) -> Result<()> {
        let mut sources = Vec::with_capacity(component.files.len());
        for file in &component.files {
            let source = asset_source(&component.id, &file.path);
            check_source(&file.content, &source)?;
            let digest = content_digest(&file.content)?.to_owned();
            sources.push((source, digest));
        }

        let component_dir = Path::new(ASSETS_DIR).join(&component.id);
        let expected = sources
            .iter()
            .map(|(source, _)| Path::new(source))
            .collect();
        self.check_holds_exactly(&component_dir, &expected)?;
        if self.verify_digests {
            for (source, digest) in &sources {
                check_digest(&self.root.join(source), digest)?;
            }
        }

        if !component.files.is_empty() {
            component.files = vec![ContentPath {
                content: ContentRef {
                    source: Some(file_url(&self.root.join(&component_dir))?),
                    ..Default::default()
                },
                path: "/".into(),
            }];
        }
        Ok(())
    }

    fn has_file(&self, path: impl AsRef<Path>) -> bool {
        self.files.contains(path.as_ref())
    }

    /// Checks that `component_dir` holds exactly the `expected` files and
    /// the directories that lead to them. With no files, `component_dir`
    /// must not exist.
    fn check_holds_exactly(&self, component_dir: &Path, expected: &HashSet<&Path>) -> Result<()> {
        if expected.is_empty() {
            ensure!(
                !self.files.contains(component_dir) && !self.dirs.contains(component_dir),
                "{} was not expected, because the component has no files",
                quoted_path(component_dir)
            );
            return Ok(());
        }
        for file in expected {
            ensure!(self.has_file(file), "{} is missing", quoted_path(file));
        }

        let expected_dirs: HashSet<&Path> = expected
            .iter()
            .flat_map(|file| {
                file.ancestors()
                    .skip(1)
                    .take_while(|ancestor| ancestor.starts_with(component_dir))
            })
            .collect();
        let unexpected_files = self
            .files
            .iter()
            .filter(|file| file.starts_with(component_dir) && !expected.contains(file.as_path()));
        let unexpected_dirs = self
            .dirs
            .iter()
            .filter(|dir| dir.starts_with(component_dir) && !expected_dirs.contains(dir.as_path()));
        if let Some(path) = unexpected_files.chain(unexpected_dirs).next() {
            bail!(
                "{} is not listed in the lock, and the guest would see it",
                quoted_path(path)
            );
        }
        Ok(())
    }
}

/// Checks that the lock records `expected` as the source of `content`.
fn check_source(content: &ContentRef, expected: &str) -> Result<()> {
    ensure!(
        content.source.as_deref() == Some(expected),
        "source {:?} should be {expected:?}",
        content.source.as_deref().unwrap_or_default()
    );
    Ok(())
}

/// Checks that the file matches its digest.
fn check_digest(path: &Path, digest: &str) -> Result<()> {
    ensure!(
        matches_digest(path, digest)?,
        "{} does not match its digest {digest}",
        quoted_path(path)
    );
    Ok(())
}

fn file_url(path: &Path) -> Result<String> {
    reqwest::Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| anyhow!("cannot make a file URL of {}", quoted_path(path)))
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use spin_common::sha256::hex_digest_from_bytes;

    use super::*;

    fn sha256_digest(bytes: &[u8]) -> String {
        format!("sha256:{}", hex_digest_from_bytes(bytes))
    }

    struct Fixture {
        _cache_dir: tempfile::TempDir,
        cache: Cache,
        base: tempfile::TempDir,
        main_wasm: Vec<u8>,
        dep_wasm: Vec<u8>,
        file: Vec<u8>,
    }

    impl Fixture {
        async fn new() -> Result<Self> {
            let cache_dir = tempfile::tempdir()?;
            let cache = Cache::new(Some(cache_dir.path().into())).await?;
            let (main_wasm, dep_wasm, file) = (
                b"main".to_vec(),
                b"dependency".to_vec(),
                b"cached file".to_vec(),
            );
            for wasm in [&main_wasm, &dep_wasm] {
                cache.write_wasm(wasm, &sha256_digest(wasm)).await?;
            }
            cache.write_data(&file, &sha256_digest(&file)).await?;
            Ok(Self {
                _cache_dir: cache_dir,
                cache,
                base: tempfile::tempdir()?,
                main_wasm,
                dep_wasm,
                file,
            })
        }

        /// Two components sharing a Wasm source; the first has a dependency,
        /// a trigger dependency, a cached file and an inline file.
        fn app(&self) -> Result<LockedApp> {
            let wasm = |bytes: &[u8]| serde_json::json!({ "content_type": "application/wasm", "digest": sha256_digest(bytes) });
            let mut app: LockedApp = serde_json::from_value(serde_json::json!({
                "spin_lock_version": 1,
                "metadata": { "origin": "vnd.fermyon.origin-oci:example.com/app:v1" },
                "triggers": [],
                "components": [
                    {
                        "id": "first",
                        "source": wasm(&self.main_wasm),
                        "dependencies": { "test:dep": { "source": wasm(&self.dep_wasm) } },
                        "trigger_dependencies": { "middleware": [{ "source": wasm(&self.dep_wasm) }] },
                        "files": [{ "digest": sha256_digest(&self.file), "path": "static/file.txt" }],
                    },
                    { "id": "second", "source": wasm(&self.main_wasm) },
                ],
            }))?;
            app.components[0]
                .files
                .push(spin_locked_app::locked::ContentPath {
                    content: ContentRef {
                        inline: Some(b"inline".to_vec()),
                        ..Default::default()
                    },
                    path: "inline.txt".into(),
                });
            Ok(app)
        }

        fn dir(&self) -> PathBuf {
            self.base.path().join("prepared")
        }

        fn read_lock(&self, dir: &Path) -> Result<LockedApp> {
            Ok(LockedApp::from_json(&std::fs::read(
                dir.join(PREPARED_LOCK_FILE),
            )?)?)
        }

        /// What is left in the base directory besides `dir`.
        fn leftovers(&self) -> Result<Vec<String>> {
            Ok(std::fs::read_dir(self.base.path())?
                .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .filter(|name| name != "prepared")
                .collect())
        }
    }

    /// Every source in the lock, Wasm and files.
    fn all_sources(app: &mut LockedApp) -> Vec<String> {
        let mut sources = vec![];
        for component in &mut app.components {
            for content in wasm_content_refs_mut(component) {
                sources.push(content.source.clone().unwrap());
            }
            for file in &component.files {
                sources.push(file.content.source.clone().unwrap());
            }
        }
        sources
    }

    #[tokio::test]
    async fn writes_a_relocatable_prepared_directory() -> Result<()> {
        let f = Fixture::new().await?;
        write_prepared(f.app()?, &f.cache, &f.dir()).await?;
        assert!(f.leftovers()?.is_empty(), "staging directory left behind");

        // Moving the directory keeps it usable: every source is relative.
        let moved = f.base.path().join("moved");
        std::fs::rename(f.dir(), &moved)?;
        let mut app = f.read_lock(&moved)?;
        assert_eq!(
            app.metadata[LAYOUT_VERSION_KEY],
            serde_json::json!(LAYOUT_VERSION)
        );
        assert_eq!(
            app.metadata["origin"],
            "vnd.fermyon.origin-oci:example.com/app:v1"
        );
        for source in all_sources(&mut app) {
            assert!(
                source.starts_with("wasm/") || source.starts_with("assets/"),
                "{source}"
            );
            assert!(moved.join(&source).is_file(), "{source}");
        }

        // Wasm: one file per digest, shared by the components that use it.
        let main_rel = format!("wasm/{}.wasm", hex_digest_from_bytes(&f.main_wasm));
        let dep_rel = format!("wasm/{}.wasm", hex_digest_from_bytes(&f.dep_wasm));
        let first = &app.components[0];
        assert_eq!(first.source.content_type, "application/wasm");
        let sources = [
            &first.source.content,
            &first.dependencies.values().next().unwrap().source.content,
            &first.trigger_dependencies["middleware"][0].source.content,
            &app.components[1].source.content,
        ];
        let expected = [
            (&main_rel, &f.main_wasm),
            (&dep_rel, &f.dep_wasm),
            (&dep_rel, &f.dep_wasm),
            (&main_rel, &f.main_wasm),
        ];
        for (content, (rel, bytes)) in sources.into_iter().zip(expected) {
            assert_eq!(content.source.as_deref(), Some(rel.as_str()));
            assert_eq!(content.digest, Some(sha256_digest(bytes)));
            assert!(content.inline.is_none());
            assert_eq!(&std::fs::read(moved.join(rel))?, bytes);
        }
        assert_eq!(std::fs::read_dir(moved.join("wasm"))?.count(), 2);

        // Files: one entry per file, with its digest, the inline one included.
        let files: Vec<_> = first
            .files
            .iter()
            .map(|file| {
                (
                    file.path.to_string_lossy().into_owned(),
                    file.content.source.clone().unwrap(),
                    file.content.digest.clone().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            files,
            [
                (
                    "static/file.txt".to_owned(),
                    "assets/first/static/file.txt".to_owned(),
                    sha256_digest(&f.file),
                ),
                (
                    "inline.txt".to_owned(),
                    "assets/first/inline.txt".to_owned(),
                    sha256_digest(b"inline"),
                ),
            ]
        );
        assert_eq!(
            std::fs::read(moved.join("assets/first/static/file.txt"))?,
            f.file
        );
        assert_eq!(
            std::fs::read(moved.join("assets/first/inline.txt"))?,
            b"inline"
        );
        assert!(app.components[1].files.is_empty());
        assert!(!moved.join("assets/second").exists());
        Ok(())
    }

    #[tokio::test]
    async fn target_must_be_a_missing_or_empty_directory() -> Result<()> {
        let f = Fixture::new().await?;
        // An empty directory is replaced.
        std::fs::create_dir(f.dir())?;
        write_prepared(f.app()?, &f.cache, &f.dir()).await?;
        assert!(f.dir().join(PREPARED_LOCK_FILE).exists());

        let file = f.base.path().join("file");
        std::fs::write(&file, "")?;
        let link = f.base.path().join("link");
        let empty = f.base.path().join("empty");
        std::fs::create_dir(&empty)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&empty, &link)?;

        let mut cases = vec![
            (f.dir(), "is not empty"),
            (file, "is not a directory"),
            (f.base.path().join(".."), "does not name a directory"),
        ];
        if cfg!(unix) {
            cases.push((link, "is a symbolic link"));
        }
        for (dir, expected) in cases {
            let err = write_prepared(f.app()?, &f.cache, &dir)
                .await
                .expect_err(expected);
            assert!(format!("{err:#}").contains(expected), "{expected}: {err:#}");
        }
        Ok(())
    }

    /// Makes a valid app invalid.
    type Breakage = fn(&mut LockedApp);

    #[tokio::test]
    async fn invalid_apps_leave_nothing_behind() -> Result<()> {
        let f = Fixture::new().await?;
        let cases: [(&str, Breakage); 9] = [
            ("invalid component id", |app| {
                app.components[1].id = "../escape".into();
            }),
            ("invalid component id", |app| {
                app.components[1].id = "first/nested".into();
            }),
            ("is used more than once", |app| {
                app.components[1].id = "first".into();
            }),
            ("is listed more than once", |app| {
                let file = app.components[0].files[0].clone();
                app.components[0].files.push(file);
            }),
            ("invalid file path", |app| {
                app.components[0].files[1].path = "../escape.txt".into();
            }),
            ("invalid file path", |app| {
                app.components[0].files[1].path = "a\\b.txt".into();
            }),
            ("unsupported Wasm digest", |app| {
                app.components[1].source.content.digest = Some("sha512:abc".into());
            }),
            ("does not match its digest", |app| {
                app.components[0].files[1].content.digest = Some(sha256_digest(b"other"));
            }),
            ("trigger dependencies outside its components", |app| {
                let mut trigger: spin_locked_app::locked::LockedTrigger =
                    serde_json::from_value(serde_json::json!({
                        "id": "t", "trigger_type": "http", "trigger_config": {},
                    }))
                    .unwrap();
                let dependency = app.components[0].trigger_dependencies["middleware"].clone();
                trigger
                    .trigger_dependencies
                    .insert("middleware".into(), dependency);
                app.triggers.push(trigger);
            }),
        ];
        for (expected, break_app) in cases {
            let mut app = f.app()?;
            break_app(&mut app);
            let err = write_prepared(app, &f.cache, &f.dir())
                .await
                .expect_err(expected);
            assert!(format!("{err:#}").contains(expected), "{expected}: {err:#}");
            assert!(!f.dir().exists(), "{expected}: directory created");
            assert!(f.leftovers()?.is_empty(), "{expected}: staging left behind");
        }
        Ok(())
    }

    #[tokio::test]
    async fn cached_content_must_be_present_and_match_its_digest() -> Result<()> {
        // Tampered Wasm, a tampered file and a missing file.
        for case in 0..3 {
            let f = Fixture::new().await?;
            match case {
                0 => {
                    f.cache
                        .write_wasm(b"tampered", &sha256_digest(&f.main_wasm))
                        .await?
                }
                1 => {
                    f.cache
                        .write_data(b"tampered", &sha256_digest(&f.file))
                        .await?
                }
                _ => std::fs::remove_file(f.cache.data_file(sha256_digest(&f.file))?)?,
            }
            let err = write_prepared(f.app()?, &f.cache, &f.dir())
                .await
                .expect_err("bad cache content should be refused");
            if case < 2 {
                // The message names the cache entry or the file, not the
                // staging directory, which is gone.
                let err = format!("{err:#}");
                assert!(err.contains("does not match its digest"), "{err}");
                assert!(!err.contains(".spin-prepare-"), "{err}");
            }
            assert!(!f.dir().exists(), "case {case}: directory created");
            assert!(
                f.leftovers()?.is_empty(),
                "case {case}: staging left behind"
            );
        }
        Ok(())
    }

    #[test]
    fn origin_is_pinned_to_the_digest() -> Result<()> {
        let digest = format!("sha256:{}", "a".repeat(64));
        assert_eq!(
            pinned_reference("example.com/org/app:v1", &digest)?,
            format!("example.com/org/app:v1@{digest}")
        );
        assert_eq!(
            pinned_reference(&format!("example.com/org/app@{digest}"), &digest)?,
            format!("example.com/org/app@{digest}")
        );
        Ok(())
    }

    #[test]
    fn wasm_paths_need_sha256_digests() {
        let hex = "a".repeat(64);
        assert_eq!(
            wasm_source(&format!("sha256:{hex}")).unwrap(),
            format!("wasm/{hex}.wasm")
        );
        for digest in [
            "".to_owned(),
            hex.clone(),
            format!("sha512:{hex}"),
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", &hex[1..]),
        ] {
            assert!(wasm_source(&digest).is_err(), "{digest:?}");
        }
    }

    /// Edits the prepared lock in `dir` as JSON.
    fn edit_lock(dir: &Path, edit: impl FnOnce(&mut serde_json::Value)) -> Result<()> {
        let path = dir.join(PREPARED_LOCK_FILE);
        let mut lock: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        edit(&mut lock);
        std::fs::write(&path, serde_json::to_vec(&lock)?)?;
        Ok(())
    }

    fn file_url(path: &Path) -> String {
        reqwest::Url::from_file_path(std::fs::canonicalize(path).unwrap())
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn loads_a_prepared_directory_in_place() -> Result<()> {
        let f = Fixture::new().await?;
        write_prepared(f.app()?, &f.cache, &f.dir()).await?;
        let moved = f.base.path().join("moved");
        std::fs::rename(f.dir(), &moved)?;

        // Files outside the component directories are not read.
        std::fs::write(moved.join("README"), "about this app")?;
        let app = load(&moved, true)?;
        let main = moved.join(format!("wasm/{}.wasm", hex_digest_from_bytes(&f.main_wasm)));
        let dep = moved.join(format!("wasm/{}.wasm", hex_digest_from_bytes(&f.dep_wasm)));
        let first = &app.components[0];
        assert_eq!(first.source.content_type, "application/wasm");
        let sources = [
            &first.source.content,
            &first.dependencies.values().next().unwrap().source.content,
            &first.trigger_dependencies["middleware"][0].source.content,
            &app.components[1].source.content,
        ];
        for (content, path) in sources.into_iter().zip([&main, &dep, &dep, &main]) {
            assert_eq!(content.source, Some(file_url(path)));
            assert!(content.digest.is_none() && content.inline.is_none());
        }
        assert_eq!(first.files.len(), 1);
        assert_eq!(first.files[0].path, Path::new("/"));
        assert_eq!(
            first.files[0].content.source,
            Some(file_url(&moved.join("assets/first")))
        );
        assert!(app.components[1].files.is_empty());
        Ok(())
    }

    /// Makes a directory tree read-only, and writable again when dropped.
    #[cfg(unix)]
    struct ReadOnly<'a>(&'a Path);

    #[cfg(unix)]
    impl<'a> ReadOnly<'a> {
        fn new(dir: &'a Path) -> Result<Self> {
            Self::set_modes(dir, 0o444, 0o555)?;
            Ok(Self(dir))
        }

        fn set_modes(dir: &Path, file_mode: u32, dir_mode: u32) -> Result<()> {
            use std::os::unix::fs::PermissionsExt;
            // Directories last when locking, first when unlocking.
            let lock = dir_mode & 0o200 == 0;
            for entry in walkdir::WalkDir::new(dir).contents_first(lock) {
                let entry = entry?;
                let mode = if entry.file_type().is_dir() {
                    dir_mode
                } else {
                    file_mode
                };
                std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(mode))?;
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    impl Drop for ReadOnly<'_> {
        fn drop(&mut self) {
            let _ = Self::set_modes(self.0, 0o644, 0o755);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn loads_a_read_only_directory_without_writing() -> Result<()> {
        let f = Fixture::new().await?;
        write_prepared(f.app()?, &f.cache, &f.dir()).await?;
        let listing = || -> Result<Vec<_>> {
            walkdir::WalkDir::new(f.dir())
                .into_iter()
                .map(|e| Ok(e?.path().to_owned()))
                .collect()
        };
        let dir = f.dir();
        let _read_only = ReadOnly::new(&dir)?;
        let before = listing()?;
        load(&f.dir(), true)?;
        assert_eq!(before, listing()?);
        Ok(())
    }

    #[tokio::test]
    async fn digests_are_checked_unless_skipped() -> Result<()> {
        let f = Fixture::new().await?;
        write_prepared(f.app()?, &f.cache, &f.dir()).await?;
        for rel in [
            format!("wasm/{}.wasm", hex_digest_from_bytes(&f.dep_wasm)),
            "assets/first/static/file.txt".to_owned(),
        ] {
            let path = f.dir().join(&rel);
            let original = std::fs::read(&path)?;
            std::fs::write(&path, b"tampered")?;
            let err = load(&f.dir(), true).expect_err("tampered content should be refused");
            assert!(
                format!("{err:#}").contains("does not match its digest"),
                "{rel}: {err:#}"
            );
            load(&f.dir(), false)?;
            std::fs::write(&path, original)?;
        }
        load(&f.dir(), true)?;
        Ok(())
    }

    /// Makes a valid prepared directory invalid.
    type DirBreakage = fn(&Path, &str, &str);

    /// Checks that each breakage of a fresh prepared directory makes `load`
    /// fail with the expected message.
    async fn assert_refused(f: &Fixture, cases: Vec<(&str, DirBreakage)>) -> Result<()> {
        let main = format!("wasm/{}.wasm", hex_digest_from_bytes(&f.main_wasm));
        let file = "assets/first/static/file.txt";
        for (i, (expected, break_dir)) in cases.into_iter().enumerate() {
            let dir = f.base.path().join(format!("case-{i}"));
            write_prepared(f.app()?, &f.cache, &dir).await?;
            break_dir(&dir, &main, file);
            let err = load(&dir, false).expect_err(expected);
            assert!(
                format!("{err:#}").contains(expected),
                "case {i}, {expected}: {err:#}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn directories_that_do_not_match_their_lock_are_refused() -> Result<()> {
        let f = Fixture::new().await?;
        let dep = format!("wasm/{}.wasm", hex_digest_from_bytes(&f.dep_wasm));
        let cases: Vec<(&str, DirBreakage)> = vec![
            ("is not a prepared application", |dir, _, _| {
                std::fs::remove_file(dir.join(PREPARED_LOCK_FILE)).unwrap();
            }),
            ("prepared layout version", |dir, _, _| {
                edit_lock(dir, |lock| lock["metadata"][LAYOUT_VERSION_KEY] = 2.into()).unwrap();
            }),
            ("should be", |dir, _, _| {
                edit_lock(dir, |lock| {
                    lock["components"][1]["source"]["source"] = "elsewhere.wasm".into()
                })
                .unwrap();
            }),
            ("should be", |dir, _, _| {
                // A file of the component, listed under another path.
                edit_lock(dir, |lock| {
                    lock["components"][0]["files"][0]["source"] = "assets/first/inline.txt".into()
                })
                .unwrap();
            }),
            ("should be", |dir, _, _| {
                // Another component's directory.
                edit_lock(dir, |lock| {
                    lock["components"][0]["files"][0]["source"] =
                        "assets/second/static/file.txt".into()
                })
                .unwrap();
            }),
            ("invalid file path", |dir, _, _| {
                edit_lock(dir, |lock| {
                    lock["components"][0]["files"][0]["path"] = "../x".into()
                })
                .unwrap();
            }),
            ("invalid file path", |dir, _, _| {
                edit_lock(dir, |lock| {
                    lock["components"][0]["files"][0]["path"] = "".into()
                })
                .unwrap();
            }),
            ("is listed more than once", |dir, _, _| {
                edit_lock(dir, |lock| {
                    let file = lock["components"][0]["files"][0].clone();
                    lock["components"][0]["files"]
                        .as_array_mut()
                        .unwrap()
                        .push(file);
                })
                .unwrap();
            }),
            ("missing expected digest", |dir, _, _| {
                edit_lock(dir, |lock| {
                    lock["components"][0]["files"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("digest");
                })
                .unwrap();
            }),
            ("invalid component id", |dir, _, _| {
                edit_lock(dir, |lock| {
                    lock["components"][1]["id"] = "first/nested".into()
                })
                .unwrap();
            }),
            ("is used more than once", |dir, _, _| {
                edit_lock(dir, |lock| lock["components"][1]["id"] = "first".into()).unwrap();
            }),
            (
                "trigger dependencies outside its components",
                |dir, _, _| {
                    edit_lock(dir, |lock| {
                        let dependency = lock["components"][0]["trigger_dependencies"].clone();
                        lock["triggers"] = serde_json::json!([{
                            "id": "t", "trigger_type": "http", "trigger_config": {},
                            "trigger_dependencies": dependency,
                        }]);
                    })
                    .unwrap();
                },
            ),
            ("is missing", |dir, main, _| {
                std::fs::remove_file(dir.join(main)).unwrap();
            }),
            ("is missing", |dir, _, file| {
                std::fs::remove_file(dir.join(file)).unwrap();
            }),
            ("is not listed", |dir, _, _| {
                std::fs::write(dir.join("assets/first/extra.txt"), "").unwrap();
            }),
            ("is not listed", |dir, _, _| {
                std::fs::create_dir(dir.join("assets/first/extra")).unwrap();
            }),
            ("is not listed", |dir, _, _| {
                std::fs::write(dir.join("assets/first/static/extra.txt"), "").unwrap();
            }),
            ("is missing", |dir, _, file| {
                std::fs::remove_file(dir.join(file)).unwrap();
                std::fs::create_dir(dir.join(file)).unwrap();
            }),
            ("was not expected", |dir, _, _| {
                std::fs::create_dir_all(dir.join("assets/second")).unwrap();
            }),
        ];
        assert_refused(&f, cases).await?;

        // A Wasm source naming another Wasm file of the directory.
        let dir = f.base.path().join("other-wasm");
        write_prepared(f.app()?, &f.cache, &dir).await?;
        edit_lock(&dir, |lock| {
            lock["components"][1]["source"]["source"] = dep.into()
        })?;
        let err = load(&dir, false).expect_err("another Wasm file");
        assert!(format!("{err:#}").contains("should be"), "{err:#}");
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symbolic_links_are_refused_below_the_directory() -> Result<()> {
        use std::os::unix::fs::symlink;
        let f = Fixture::new().await?;

        /// Moves `rel` out of `dir` and puts a symbolic link to it in its place.
        fn replace_with_link(dir: &Path, rel: &str) {
            let outside = dir.with_extension(rel.replace('/', "-"));
            std::fs::rename(dir.join(rel), &outside).unwrap();
            symlink(&outside, dir.join(rel)).unwrap();
        }

        let cases: Vec<(&str, DirBreakage)> = vec![
            ("symbolic link", |dir, _, _| {
                replace_with_link(dir, PREPARED_LOCK_FILE);
            }),
            ("symbolic link", |dir, main, _| {
                replace_with_link(dir, main);
            }),
            ("symbolic link", |dir, _, file| {
                replace_with_link(dir, file);
            }),
            ("symbolic link", |dir, _, _| replace_with_link(dir, "wasm")),
            ("symbolic link", |dir, _, _| {
                replace_with_link(dir, "assets")
            }),
            ("symbolic link", |dir, _, _| {
                replace_with_link(dir, "assets/first");
            }),
            ("symbolic link", |dir, _, _| {
                replace_with_link(dir, "assets/first/static");
            }),
            ("symbolic link", |dir, _, _| {
                symlink("/", dir.join("assets/first/root")).unwrap();
            }),
            ("symbolic link", |dir, _, _| {
                // Outside the component directories too.
                symlink("/", dir.join("README")).unwrap();
            }),
        ];
        assert_refused(&f, cases).await?;

        // `dir` itself may be a link: it names the directory to use.
        write_prepared(f.app()?, &f.cache, &f.dir()).await?;
        let link = f.base.path().join("link");
        symlink(f.dir(), &link)?;
        load(&link, true)?;
        Ok(())
    }
}
