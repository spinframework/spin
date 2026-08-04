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
        let locked_content = tokio::fs::read(&lockfile_path)
            .await
            .with_context(|| format!("failed to read from {}", quoted_path(&lockfile_path)))?;
        let locked_app_descriptor = locked_app_descriptor(&manifest)?;
        verify_digest(&locked_content, &locked_app_descriptor.digest).with_context(|| {
            format!(
                "{} is not the locked app of {reference:?}",
                quoted_path(&lockfile_path)
            )
        })?;
        let locked_json: serde_json::Value = serde_json::from_slice(&locked_content)
            .with_context(|| format!("OCI config {} is not JSON", quoted_path(&lockfile_path)))?;

        if locked_json.get("spin_lock_version").is_some() {
            let mut locked_app = LockedApp::from_json(&locked_content).with_context(|| {
                format!(
                    "failed to decode locked app from {}",
                    quoted_path(&lockfile_path)
                )
            })?;

            // Update origin metadata
            let resolved_reference = Reference::try_from(reference).context("invalid reference")?;
            let origin_uri = format!("{ORIGIN_URL_SCHEME}:{resolved_reference}");
            locked_app
                .metadata
                .insert("origin".to_string(), origin_uri.into());

            for component in &mut locked_app.components {
                self.resolve_component_content_refs(component, cache)
                    .await
                    .with_context(|| {
                        format!("failed to resolve content for component {:?}", component.id)
                    })?;
            }
            Ok(ExecutableArtifact::Application(locked_app))
        } else {
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

        // Update wasm content path
        let wasm_digest = content_digest(&component.source.content)?;
        let wasm_path = cache.wasm_file(wasm_digest)?;
        component.source.content = content_ref(wasm_path)?;

        for dep in &mut component.dependencies.values_mut() {
            let dep_wasm_digest = content_digest(&dep.source.content)?;
            let dep_wasm_path = cache.wasm_file(dep_wasm_digest)?;
            dep.source.content = content_ref(dep_wasm_path)?;
        }

        // Applications pushed without composition carry their trigger dependencies
        // (e.g. HTTP middleware) as layers of their own, so those need resolving too.
        for deps in component.trigger_dependencies.values_mut() {
            for dep in deps {
                let dep_wasm_digest = content_digest(&dep.source.content)?;
                let dep_wasm_path = cache.wasm_file(dep_wasm_digest)?;
                dep.source.content = content_ref(dep_wasm_path)?;
            }
        }

        let mount_dir = self.working_dir.join("assets").join(&component.id);

        if !component.files.is_empty() {
            let mount_dir = mount_dir.clone();
            resolve_content_paths(cache, &mount_dir, &component.files).await?;
            component.files = vec![ContentPath {
                content: content_ref(mount_dir)?,
                path: "/".into(),
            }];
        }

        // This will exist for non-precomposed apps
        for dep in component.dependencies.values_mut() {
            if let spin_locked_app::locked::InheritConfiguration::Exact(caps) = &mut dep.inherit {
                let mount_dir = mount_dir
                    .join("_deps_assets")
                    .join(caps.capabilities_key.to_string());
                resolve_content_paths(cache, &mount_dir, &caps.files).await?;
                caps.files = vec![ContentPath {
                    content: content_ref(mount_dir)?,
                    path: "/".into(),
                }];
            }
        }

        // This will exist for precomposed apps
        for dep_cap in &mut component.precomposed_dependency_capabilities {
            let mount_dir = mount_dir
                .join("_deps_assets")
                .join(dep_cap.capabilities_key.to_string());
            resolve_content_paths(cache, &mount_dir, &dep_cap.files).await?;
            dep_cap.files = vec![ContentPath {
                content: content_ref(mount_dir)?,
                path: "/".into(),
            }];
        }

        Ok(())
    }
}

async fn resolve_content_paths(
    cache: &Cache,
    mount_dir: &Path,
    files: &Vec<ContentPath>,
) -> Result<(), anyhow::Error> {
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

    Ok(())
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
            precomposed_dependency_capabilities: Default::default(),
        };

        loader
            .resolve_component_content_refs(&mut component, &cache)
            .await
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
