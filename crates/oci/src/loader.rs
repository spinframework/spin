use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, ensure};
use oci_client::Reference;
use reqwest::Url;
use spin_common::ui::quoted_path;
use spin_loader::cache::Cache;
use spin_locked_app::locked::{ContentPath, ContentRef, LockedApp, LockedComponent};

use crate::client::{locked_app_descriptor, verify_digest};
use crate::{Client, ORIGIN_URL_SCHEME};

/// OciLoader loads an OCI app in preparation for running with Spin.
pub struct OciLoader {
    working_dir: PathBuf,
}

/// The artifact loaded by the `OciLoader`
pub enum ExecutableArtifact {
    /// The OCI reference contained a Spin application
    Application(LockedApp),
    /// The OCI reference contained a Wasm package
    Package(PathBuf),
}

impl OciLoader {
    /// Creates a new OciLoader which builds temporary mount directory(s) in
    /// the given working_dir.
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        let working_dir = working_dir.into();
        Self { working_dir }
    }

    /// Pulls and loads an OCI Artifact and returns a LockedApp with the given OCI client and reference
    pub async fn load_app(
        &self,
        client: &mut Client,
        reference: &str,
    ) -> Result<ExecutableArtifact> {
        // Fetch app
        let (manifest, digest) = client.pull_with_digest(reference).await.with_context(|| {
            format!("cannot pull Spin application from registry reference {reference:?}")
        })?;

        // Read the locked app cached for the pulled digest. The location for the
        // reference as given may hold another artifact if the reference is a tag.
        let parsed = Reference::try_from(reference).context("invalid reference")?;
        let pinned = Reference::with_digest(
            parsed.registry().to_owned(),
            parsed.repository().to_owned(),
            digest,
        );
        let lockfile_path = client
            .lockfile_path(pinned.whole())
            .await
            .context("cannot get path to spin.lock")?;
        self.load_from_cache(manifest, lockfile_path, reference, &client.cache)
            .await
    }

    /// Loads an OCI Artifact from the given cache and returns a LockedApp with the given reference
    pub async fn load_from_cache(
        &self,
        manifest: oci_client::manifest::OciImageManifest,
        lockfile_path: PathBuf,
        reference: &str,
        cache: &Cache,
    ) -> std::result::Result<ExecutableArtifact, anyhow::Error> {
        match read_locked_app(&manifest, &lockfile_path, reference).await? {
            Some(mut locked_app) => {
                for component in &mut locked_app.components {
                    self.resolve_component_content_refs(component, cache)
                        .await
                        .with_context(|| {
                            format!("failed to resolve content for component {:?}", component.id)
                        })?;
                }
                Ok(ExecutableArtifact::Application(locked_app))
            }
            None => {
                if manifest.layers.len() != 1 {
                    anyhow::bail!(
                        "expected single layer in OCI package, found {} layers",
                        manifest.layers.len()
                    );
                }
                let layer = &manifest.layers[0]; // guaranteed safe by previous check
                let wasm_path = cache.wasm_path(&layer.digest);
                Ok(ExecutableArtifact::Package(wasm_path))
            }
        }
    }

    /// Resolves all digest references in the component (including source, dependencies,
    /// and asset files). Wasm sources are replaced with cache paths. Asset files are
    /// collected to a mount path, and the component `files` amended to reflect that.
    ///
    /// This function assumes that:
    ///
    /// 1. All resolvable items have digest or inline sources.
    /// 2. All digest-addressed content is already in the cache.
    ///
    /// If either of these is not true, the function errors.
    pub async fn resolve_component_content_refs(
        &self,
        component: &mut LockedComponent,
        cache: &Cache,
    ) -> Result<()> {
        ensure!(
            is_safe_to_join(&component.id),
            "invalid component id {}",
            component.id
        );

        // Update wasm content paths
        for content in wasm_content_refs_mut(component) {
            let wasm_path = cache.wasm_file(content_digest(content)?)?;
            *content = content_ref(wasm_path)?;
        }

        if !component.files.is_empty() {
            let mount_dir = write_component_files(
                &component.id,
                &component.files,
                cache,
                &self.working_dir.join("assets"),
            )
            .await?;
            component.files = vec![ContentPath {
                content: content_ref(mount_dir)?,
                path: "/".into(),
            }]
        }

        Ok(())
    }
}

/// Reads the locked app of an OCI artifact from `lockfile_path`, after
/// checking it against the digest the manifest records, and sets its origin
/// to `reference`. Returns `None` if the artifact is a Wasm package rather
/// than a Spin application.
pub(crate) async fn read_locked_app(
    manifest: &oci_client::manifest::OciImageManifest,
    lockfile_path: &Path,
    reference: &str,
) -> Result<Option<LockedApp>> {
    let locked_content = tokio::fs::read(lockfile_path)
        .await
        .with_context(|| format!("failed to read from {}", quoted_path(lockfile_path)))?;
    let locked_app_descriptor = locked_app_descriptor(manifest)?;
    verify_digest(&locked_content, &locked_app_descriptor.digest).with_context(|| {
        format!(
            "{} is not the locked app of {reference:?}",
            quoted_path(lockfile_path)
        )
    })?;
    let locked_json: serde_json::Value = serde_json::from_slice(&locked_content)
        .with_context(|| format!("OCI config {} is not JSON", quoted_path(lockfile_path)))?;
    if locked_json.get("spin_lock_version").is_none() {
        return Ok(None);
    }

    let mut locked_app = LockedApp::from_json(&locked_content).with_context(|| {
        format!(
            "failed to decode locked app from {}",
            quoted_path(lockfile_path)
        )
    })?;

    // Update origin metadata
    let resolved_reference = Reference::try_from(reference).context("invalid reference")?;
    let origin_uri = format!("{ORIGIN_URL_SCHEME}:{resolved_reference}");
    locked_app
        .metadata
        .insert("origin".to_string(), origin_uri.into());
    Ok(Some(locked_app))
}

/// Returns each Wasm content reference of the component: its own source,
/// and those of its dependencies and trigger dependencies.
///
/// Applications pushed without composition carry their trigger dependencies
/// (e.g. HTTP middleware) as layers of their own, so those are included.
pub(crate) fn wasm_content_refs_mut(
    component: &mut LockedComponent,
) -> impl Iterator<Item = &mut ContentRef> {
    std::iter::once(&mut component.source.content)
        .chain(
            component
                .dependencies
                .values_mut()
                .map(|dep| &mut dep.source.content),
        )
        .chain(
            component
                .trigger_dependencies
                .values_mut()
                .flatten()
                .map(|dep| &mut dep.source.content),
        )
}

/// Writes a component's files, from their inline content or the cache, to
/// `assets_dir/<component id>`, laid out as the guest sees them, and returns
/// that directory.
///
/// Every file must have inline content or a digest whose content is in the
/// cache. Nothing is created when `files` is empty.
pub(crate) async fn write_component_files(
    component_id: &str,
    files: &[ContentPath],
    cache: &Cache,
    assets_dir: &Path,
) -> Result<PathBuf> {
    // Callers may check the id too; this function joins it to a path, so it
    // checks for itself.
    ensure!(
        is_safe_to_join(component_id),
        "invalid component id {component_id}"
    );
    let mount_dir = assets_dir.join(component_id);
    for file in files {
        ensure!(is_safe_to_join(&file.path), "invalid file mount {file:?}");
        let mount_path = mount_dir.join(&file.path);

        // Create parent directory
        let mount_parent = mount_path
            .parent()
            .with_context(|| format!("invalid mount path {mount_path:?}"))?;
        tokio::fs::create_dir_all(mount_parent)
            .await
            .with_context(|| format!("failed to create temporary mount path {mount_path:?}"))?;

        if let Some(content_bytes) = file.content.inline.as_deref() {
            // Write inline content to disk
            tokio::fs::write(&mount_path, content_bytes)
                .await
                .with_context(|| format!("failed to write inline content to {mount_path:?}"))?;
        } else {
            // Copy content
            let digest = content_digest(&file.content)?;
            let content_path = cache.data_file(digest)?;
            // TODO: parallelize
            tokio::fs::copy(&content_path, &mount_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to copy {}->{mount_path:?}",
                        quoted_path(&content_path)
                    )
                })?;
        }
    }
    Ok(mount_dir)
}

fn content_digest(content_ref: &ContentRef) -> Result<&str> {
    content_ref
        .digest
        .as_deref()
        .with_context(|| format!("content missing expected digest: {content_ref:?}"))
}

fn content_ref(path: impl AsRef<Path>) -> Result<ContentRef> {
    let path = std::fs::canonicalize(path)?;
    let url = Url::from_file_path(path).map_err(|_| anyhow!("couldn't build file URL"))?;
    Ok(ContentRef {
        source: Some(url.to_string()),
        ..Default::default()
    })
}

fn is_safe_to_join(path: impl AsRef<Path>) -> bool {
    // This could be loosened, but currently should always be true
    path.as_ref()
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

#[cfg(test)]
mod test {
    use super::*;

    fn locked_app_json(component: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "spin_lock_version": 1,
            "metadata": { "name": component },
            "triggers": [],
            "components": [],
        }))
        .unwrap()
    }

    fn manifest_with_locked_app(locked_app: &[u8]) -> oci_client::manifest::OciImageManifest {
        let digest = format!(
            "sha256:{}",
            spin_common::sha256::hex_digest_from_bytes(locked_app)
        );
        oci_client::manifest::OciImageManifest {
            layers: vec![oci_client::manifest::OciDescriptor {
                media_type: crate::client::SPIN_APPLICATION_MEDIA_TYPE.to_owned(),
                digest,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    async fn mount_test_component_assets(
        component_id: &str,
        working_dir: &Path,
        cache_dir: &Path,
    ) -> anyhow::Result<()> {
        let loader = OciLoader::new(working_dir);
        let cache = Cache::new(Some(cache_dir.into())).await?;

        let fake_wasm = vec![1, 2, 3, 4];
        let fake_wasm_digest = spin_common::sha256::hex_digest_from_bytes(&fake_wasm);
        cache.write_wasm(&fake_wasm, &fake_wasm_digest).await?;

        let fake_file = "some content".into();
        let fake_file_digest = spin_common::sha256::hex_digest_from_bytes(&fake_file);
        cache.write_data(&fake_file, &fake_file_digest).await?;

        let mut component = LockedComponent {
            id: component_id.into(),
            metadata: Default::default(),
            source: spin_locked_app::locked::LockedComponentSource {
                content_type: "application/wasm".into(),
                content: ContentRef {
                    source: None,
                    inline: Some(fake_wasm),
                    digest: Some(fake_wasm_digest),
                },
            },
            env: Default::default(),
            files: vec![ContentPath {
                content: ContentRef {
                    source: None,
                    inline: Some(fake_file),
                    digest: Some(fake_file_digest),
                },
                path: "file.txt".into(),
            }],
            config: Default::default(),
            dependencies: Default::default(),
            trigger_dependencies: Default::default(),
            host_requirements: Default::default(),
        };

        loader
            .resolve_component_content_refs(&mut component, &cache)
            .await
    }

    fn wasm_source(digest: &str) -> serde_json::Value {
        serde_json::json!({ "content_type": "application/wasm", "digest": digest })
    }

    fn sha256_digest(bytes: &[u8]) -> String {
        format!(
            "sha256:{}",
            spin_common::sha256::hex_digest_from_bytes(bytes)
        )
    }

    #[tokio::test]
    async fn content_refs_resolve_to_cache_and_assets() -> anyhow::Result<()> {
        let working_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let cache = Cache::new(Some(cache_dir.path().into())).await?;
        let loader = OciLoader::new(working_dir.path());

        let mut wasm_digests = vec![];
        for wasm in [b"main".as_slice(), b"dependency", b"middleware"] {
            let digest = sha256_digest(wasm);
            cache.write_wasm(wasm, &digest).await?;
            wasm_digests.push(digest);
        }
        let file = b"some content".to_vec();
        let file_digest = sha256_digest(&file);
        cache.write_data(&file, &file_digest).await?;
        let inline = b"inline content".to_vec();

        let mut component: LockedComponent = serde_json::from_value(serde_json::json!({
            "id": "comp",
            "source": wasm_source(&wasm_digests[0]),
            "dependencies": { "test:dep": { "source": wasm_source(&wasm_digests[1]) } },
            "trigger_dependencies": {
                "middleware": [{ "source": wasm_source(&wasm_digests[2]) }]
            },
            "files": [{ "digest": file_digest, "path": "dir/file.txt" }],
        }))?;
        component.files.push(ContentPath {
            content: ContentRef {
                inline: Some(inline.clone()),
                ..Default::default()
            },
            path: "inline.txt".into(),
        });

        loader
            .resolve_component_content_refs(&mut component, &cache)
            .await?;

        // Wasm sources point at the cache, with no digest or inline content.
        let wasm_contents = [
            &component.source.content,
            &component
                .dependencies
                .values()
                .next()
                .unwrap()
                .source
                .content,
            &component.trigger_dependencies["middleware"][0]
                .source
                .content,
        ];
        for (content, digest) in wasm_contents.into_iter().zip(&wasm_digests) {
            let expected = content_ref(cache.wasm_file(digest)?)?;
            assert_eq!(content.source, expected.source);
            assert!(content.digest.is_none() && content.inline.is_none());
        }

        // Files are written beneath assets/<component id> and mounted at `/`.
        let mount_dir = working_dir.path().join("assets/comp");
        assert_eq!(component.files.len(), 1);
        assert_eq!(component.files[0].path, Path::new("/"));
        assert_eq!(
            component.files[0].content.source,
            content_ref(&mount_dir)?.source
        );
        assert_eq!(std::fs::read(mount_dir.join("dir/file.txt"))?, file);
        assert_eq!(std::fs::read(mount_dir.join("inline.txt"))?, inline);

        Ok(())
    }

    #[tokio::test]
    async fn file_path_cannot_be_used_to_escape_asset_directory() -> anyhow::Result<()> {
        let base_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let cache = Cache::new(Some(cache_dir.path().into())).await?;
        let assets_dir = base_dir.path().join("good/assets");

        let files: Vec<ContentPath> = serde_json::from_value(serde_json::json!([
            { "inline": "", "path": "../../escaped.txt" },
        ]))?;
        write_component_files("comp", &files, &cache, &assets_dir)
            .await
            .expect_err("a file path with .. should be rejected");
        assert!(!base_dir.path().join("good/escaped.txt").exists());
        assert!(!base_dir.path().join("escaped.txt").exists());

        Ok(())
    }

    #[tokio::test]
    async fn locked_app_of_another_artifact_is_rejected() -> anyhow::Result<()> {
        let working_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let cache = Cache::new(Some(cache_dir.path().into())).await?;
        let loader = OciLoader::new(working_dir.path());
        let reference = "example.com/org/app@sha256:0a867093096e0ef01ef749b12b6e7a90e4952eda107f89a676eeedce63a8361f";

        let requested = locked_app_json("requested");
        let lockfile_path = working_dir.path().join("config.json");

        // The cached locked app belongs to another artifact.
        std::fs::write(&lockfile_path, locked_app_json("other"))?;
        let err = loader
            .load_from_cache(
                manifest_with_locked_app(&requested),
                lockfile_path.clone(),
                reference,
                &cache,
            )
            .await
            .err()
            .expect("a locked app that does not match the manifest should be rejected");
        assert!(
            format!("{err:#}").contains("is not the locked app of"),
            "unexpected error: {err:#}"
        );

        // The cached locked app is the requested one.
        std::fs::write(&lockfile_path, &requested)?;
        let artifact = loader
            .load_from_cache(
                manifest_with_locked_app(&requested),
                lockfile_path,
                reference,
                &cache,
            )
            .await?;
        assert!(matches!(artifact, ExecutableArtifact::Application(_)));

        Ok(())
    }

    #[tokio::test]
    async fn component_id_cannot_be_used_to_escape_asset_directory() -> anyhow::Result<()> {
        let component_id = "../../crimeburger";

        let base_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let working_dir = base_dir.path().join("good");

        let res = mount_test_component_assets(component_id, &working_dir, cache_dir.path()).await;

        // Validate that the file was not written - checking the error is insufficient
        // because it could write and then fail.
        assert!(
            !base_dir.path().join("crimeburger/file.txt").exists(),
            "crimeburger file exists when it shouldn't"
        );
        res.expect_err("illegal component id should have caused error");

        Ok(())
    }

    #[tokio::test]
    async fn legal_component_id_is_accepted() -> anyhow::Result<()> {
        let component_id = "legal";

        let base_dir = tempfile::tempdir()?;
        let cache_dir = tempfile::tempdir()?;
        let working_dir = base_dir.path().join("good");

        mount_test_component_assets(component_id, &working_dir, cache_dir.path()).await?;

        assert!(
            working_dir.join("assets/legal/file.txt").exists(),
            "file was not written when it should be"
        );

        Ok(())
    }
}
