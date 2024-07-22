use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, ensure, Context, Result};
use futures::{future::try_join_all, StreamExt};
use reqwest::Url;
use spin_common::{paths::parent_dir, sloth, ui::quoted_path};
use spin_locked_app::{
    locked::{
        self, ContentPath, ContentRef, LockedApp, LockedComponent, LockedComponentDependency,
        LockedComponentSource, LockedTrigger,
    },
    values::{ValuesMap, ValuesMapBuilder},
};
use spin_manifest::schema::v2::{self, AppManifest, KebabId, WasiFilesMount};
use spin_outbound_networking_config::allowed_hosts::{
    AllowedHostsConfig, SERVICE_CHAINING_DOMAIN_SUFFIX,
};
use spin_serde::DependencyName;
use std::collections::BTreeMap;
use tokio::{io::AsyncWriteExt, sync::Semaphore};

use crate::{cache::Cache, FilesMountStrategy};

#[derive(Debug)]
pub struct LocalLoader {
    app_root: PathBuf,
    files_mount_strategy: FilesMountStrategy,
    file_loading_permits: std::sync::Arc<Semaphore>,
    wasm_loader: WasmLoader,
}

impl LocalLoader {
    pub async fn new(
        app_root: &Path,
        files_mount_strategy: FilesMountStrategy,
        cache_root: Option<PathBuf>,
    ) -> Result<Self> {
        let app_root = safe_canonicalize(app_root)
            .with_context(|| format!("Invalid manifest dir `{}`", app_root.display()))?;
        let file_loading_permits =
            std::sync::Arc::new(Semaphore::new(crate::MAX_FILE_LOADING_CONCURRENCY));
        Ok(Self {
            app_root: app_root.clone(),
            files_mount_strategy,
            // Limit concurrency to avoid hitting system resource limits
            file_loading_permits: file_loading_permits.clone(),
            wasm_loader: WasmLoader::new(app_root, cache_root, Some(file_loading_permits)).await?,
        })
    }

    // Load the manifest file (spin.toml) at the given path into a LockedApp,
    // preparing all its content for execution.
    pub async fn load_file(&self, path: impl AsRef<Path>) -> Result<LockedApp> {
        // Parse manifest
        let path = path.as_ref();
        let manifest = spin_manifest::manifest_from_file(path).with_context(|| {
            format!(
                "Failed to read Spin app manifest from {}",
                quoted_path(path)
            )
        })?;
        let mut locked = self
            .load_manifest(manifest)
            .await
            .with_context(|| format!("Failed to load Spin app from {}", quoted_path(path)))?;

        // Set origin metadata
        locked
            .metadata
            .insert("origin".into(), file_url(path)?.into());

        Ok(locked)
    }

    // Load the given manifest into a LockedApp, ready for execution.
    pub(crate) async fn load_manifest(&self, mut manifest: AppManifest) -> Result<LockedApp> {
        spin_manifest::normalize::normalize_manifest(&mut manifest);

        manifest.validate_dependencies()?;

        let AppManifest {
            spin_manifest_version: _,
            application,
            variables,
            triggers,
            components,
        } = manifest;

        let metadata = locked_metadata(application, triggers.keys().cloned())?;

        let variables = variables
            .into_iter()
            .map(|(name, v)| Ok((name.to_string(), locked_variable(v)?)))
            .collect::<Result<_>>()?;

        let triggers = triggers
            .into_iter()
            .flat_map(|(trigger_type, configs)| {
                configs
                    .into_iter()
                    .map(|trigger| locked_trigger(trigger_type.clone(), trigger))
                    .collect::<Vec<_>>()
            })
            .collect::<Result<Vec<_>>>()?;

        let sloth_guard = warn_if_component_load_slothful();

        // Load all components concurrently
        let components = try_join_all(components.into_iter().map(|(id, c)| async move {
            self.load_component(&id, c)
                .await
                .with_context(|| format!("Failed to load component `{id}`"))
        }))
        .await?;

        let host_requirements = spin_locked_app::values::ValuesMap::new();

        let mut must_understand = vec![];
        if !host_requirements.is_empty() {
            must_understand.push(spin_locked_app::locked::MustUnderstand::HostRequirements);
        }
        if components.iter().any(|c| !c.host_requirements.is_empty()) {
            must_understand
                .push(spin_locked_app::locked::MustUnderstand::ComponentHostRequirements);
        }

        drop(sloth_guard);

        Ok(LockedApp {
            spin_lock_version: Default::default(),
            metadata,
            must_understand,
            host_requirements,
            variables,
            triggers,
            components,
        })
    }

    // Load the given component into a LockedComponent, ready for execution.
    async fn load_component(
        &self,
        id: &KebabId,
        component: v2::Component,
    ) -> Result<LockedComponent> {
        let allowed_outbound_hosts = component
            .normalized_allowed_outbound_hosts()
            .context("`allowed_http_hosts` is malformed")?;
        AllowedHostsConfig::validate(&allowed_outbound_hosts)
            .context("`allowed_outbound_hosts` is malformed")?;

        let component_requires_service_chaining = requires_service_chaining(&component);

        let metadata = ValuesMapBuilder::new()
            .string("description", component.description)
            .string_array("allowed_outbound_hosts", allowed_outbound_hosts)
            .string_array("key_value_stores", component.key_value_stores)
            .string_array("databases", component.sqlite_databases)
            .string_array("ai_models", component.ai_models)
            .serializable("build", component.build)?
            .take();

        let source = self
            .load_component_source(id, component.source.clone())
            .await
            .with_context(|| format!("Failed to load Wasm source {}", component.source))?;

        let dependencies = self
            .load_component_dependencies(
                id,
                component.dependencies_inherit_configuration,
                &component.dependencies,
            )
            .await?;

        let env = component.environment.into_iter().collect();

        let files = if component.files.is_empty() {
            vec![]
        } else {
            match &self.files_mount_strategy {
                FilesMountStrategy::Copy(files_mount_root) => {
                    let component_mount_root = files_mount_root.join(id.as_ref());
                    // Copy mounted files into component mount root, concurrently
                    try_join_all(component.files.iter().map(|f| {
                        self.copy_file_mounts(f, &component_mount_root, &component.exclude_files)
                    }))
                    .await?;

                    // All component files (copies) are in `component_mount_root` now
                    vec![ContentPath {
                        content: file_content_ref(component_mount_root)?,
                        path: "/".into(),
                    }]
                }
                FilesMountStrategy::Direct => {
                    ensure!(
                        component.exclude_files.is_empty(),
                        "Cannot load a component with `exclude_files` using --direct-mounts"
                    );
                    let mut files = vec![];
                    for mount in &component.files {
                        // Validate (and canonicalize) direct mount directory
                        files.push(self.resolve_direct_mount(mount).await?);
                    }
                    files
                }
            }
        };

        let config = component
            .variables
            .into_iter()
            .map(|(k, v)| (k.into(), v))
            .collect();

        let mut host_requirements = ValuesMapBuilder::new();
        if component_requires_service_chaining {
            host_requirements.string(
                spin_locked_app::locked::SERVICE_CHAINING_KEY,
                spin_locked_app::locked::HOST_REQ_REQUIRED,
            );
        }
        let host_requirements = host_requirements.build();

        Ok(LockedComponent {
            id: id.as_ref().into(),
            metadata,
            source,
            env,
            files,
            config,
            dependencies,
            host_requirements,
        })
    }

    async fn load_component_dependencies(
        &self,
        id: &KebabId,
        inherit_configuration: bool,
        dependencies: &v2::ComponentDependencies,
    ) -> Result<BTreeMap<DependencyName, LockedComponentDependency>> {
        Ok(try_join_all(dependencies.inner.iter().map(
            |(dependency_name, dependency)| async move {
                let locked_dependency = self
                    .load_component_dependency(
                        inherit_configuration,
                        dependency_name.clone(),
                        dependency.clone(),
                    )
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to load component dependency `{dependency_name}` for `{id}`"
                        )
                    })?;

                anyhow::Ok((dependency_name.clone(), locked_dependency))
            },
        ))
        .await?
        .into_iter()
        .collect())
    }

    async fn load_component_dependency(
        &self,
        inherit_configuration: bool,
        dependency_name: DependencyName,
        dependency: v2::ComponentDependency,
    ) -> Result<LockedComponentDependency> {
        let (content, export) = self
            .wasm_loader
            .load_component_dependency(&dependency_name, &dependency)
            .await?;

        Ok(LockedComponentDependency {
            source: LockedComponentSource {
                content_type: "application/wasm".into(),
                content: file_content_ref(content)?,
            },
            export,
            inherit: if inherit_configuration {
                locked::InheritConfiguration::All
            } else {
                locked::InheritConfiguration::Some(vec![])
            },
        })
    }

    // Load a Wasm source from the given ContentRef and update the source
    // URL with an absolute path to the content.
    async fn load_component_source(
        &self,
        component_id: &KebabId,
        source: v2::ComponentSource,
    ) -> Result<LockedComponentSource> {
        let path = self
            .wasm_loader
            .load_component_source(component_id.as_ref(), &source)
            .await?;
        Ok(LockedComponentSource {
            content_type: "application/wasm".into(),
            content: file_content_ref(path)?,
        })
    }

    // Copy content(s) from the given `mount`
    async fn copy_file_mounts(
        &self,
        mount: &WasiFilesMount,
        dest_root: &Path,
        exclude_files: &[String],
    ) -> Result<()> {
        match mount {
            WasiFilesMount::Pattern(pattern) => {
                self.copy_glob_or_path(pattern, dest_root, exclude_files)
                    .await
            }
            WasiFilesMount::Placement {
                source,
                destination,
            } => {
                let src = Path::new(source);
                let dest = dest_root.join(destination.trim_start_matches('/'));
                self.copy_file_or_directory(src, &dest, destination, exclude_files)
                    .await
            }
        }
    }

    // Copy files matching glob pattern or single file/directory path.
    async fn copy_glob_or_path(
        &self,
        glob_or_path: &str,
        dest_root: &Path,
        exclude_files: &[String],
    ) -> Result<()> {
        if glob_or_path == ".." || glob_or_path.ends_with("/..") {
            bail!("A file pattern can't end in a parent directory path (..)\nIf you want to include a directory, use source-destination form, or a glob pattern ending in **/*.\nLearn more: https://spinframework.dev/writing-apps#including-files-with-components");
        }
        if glob_or_path == "." || glob_or_path.ends_with("/.") {
            bail!("A file pattern can't end in a current directory path (.)\nIf you want to include a directory, use source-destination form, or a glob pattern ending in **/*.\nLearn more: https://spinframework.dev/writing-apps#including-files-with-components");
        }

        if glob_or_path == "*" {
            tracing::warn!(alert_in_dev = true, "A component is including the entire application directory as asset files. This is unlikely to be what you want.\nIf this is what you want, use the pattern \"./*\" to avoid this warning.\nLearn more: https://spinframework.dev/writing-apps#including-files-with-components\n");
        }
        if glob_or_path == "**/*" {
            tracing::warn!(alert_in_dev = true, "A component is including the entire application directory tree as asset files. This is unlikely to be what you want.\nIf this is what you want, use the pattern \"./**/*\" to avoid this warning.\nLearn more: https://spinframework.dev/writing-apps#including-files-with-components\n");
        }

        let path = self.app_root.join(glob_or_path);
        if path.exists() {
            let dest = dest_root.join(glob_or_path);
            if path.is_dir() {
                // "single/dir"
                let pattern = path.join("**/*");
                self.copy_glob(&pattern, &self.app_root, &dest, exclude_files)
                    .await?;
            } else {
                // "single/file.txt"
                self.copy_single_file(&path, &dest, glob_or_path).await?;
            }
        } else if looks_like_glob_pattern(glob_or_path) {
            // "glob/pattern/*"
            self.copy_glob(&path, &self.app_root, dest_root, exclude_files)
                .await?;
        } else {
            bail!("{glob_or_path:?} does not exist and doesn't appear to be a glob pattern");
        }
        Ok(())
    }

    // Copy a single file or entire directory from `src` to `dest`
    async fn copy_file_or_directory(
        &self,
        src: &Path,
        dest: &Path,
        guest_dest: &str,
        exclude_files: &[String],
    ) -> Result<()> {
        let src_path = self.app_root.join(src);
        let meta = crate::fs::metadata(&src_path)
            .await
            .map_err(|e| explain_file_mount_source_error(e, src))?;
        if meta.is_dir() {
            // { source = "host/dir", destination = "guest/dir" }
            let pattern = src_path.join("**/*");
            self.copy_glob(&pattern, &src_path, dest, exclude_files)
                .await?;
        } else {
            // { source = "host/file.txt", destination = "guest/file.txt" }
            self.copy_single_file(&src_path, dest, guest_dest).await?;
        }
        Ok(())
    }

    // Copy files matching glob `pattern` into `dest_root`.
    async fn copy_glob(
        &self,
        pattern: &Path,
        src_prefix: &Path,
        dest_root: &Path,
        exclude_files: &[String],
    ) -> Result<()> {
        let pattern = pattern
            .to_str()
            .with_context(|| format!("invalid (non-utf8) file pattern {pattern:?}"))?;

        let paths = glob::glob(pattern)
            .with_context(|| format!("Failed to resolve glob pattern {pattern:?}"))?;

        let exclude_patterns = exclude_files
            .iter()
            .map(|pattern| {
                glob::Pattern::new(pattern)
                    .with_context(|| format!("Invalid exclude_files glob pattern {pattern:?}"))
            })
            .collect::<Result<Vec<_>>>()?;

        crate::fs::create_dir_all(dest_root)
            .await
            .with_context(|| {
                format!(
                    "Failed to create parent directory {}",
                    quoted_path(&dest_root)
                )
            })?;

        for path_res in paths {
            let src = path_res?;
            if !src.is_file() {
                continue;
            }

            let Ok(app_root_path) = src.strip_prefix(&self.app_root) else {
                bail!("{pattern} cannot be mapped because it is outside the application directory. Files must be within the application directory.");
            };

            if exclude_patterns
                .iter()
                .any(|pattern| pattern.matches_path(app_root_path))
            {
                tracing::debug!(
                    "File {app_root_path:?} excluded by exclude_files {exclude_files:?}"
                );
                continue;
            }

            let relative_path = src.strip_prefix(src_prefix)?;
            let dest = dest_root.join(relative_path);
            self.copy_single_file(&src, &dest, &relative_path.to_string_lossy())
                .await?;
        }
        Ok(())
    }

    // Copy a single file from `src` to `dest`, creating parent directories.
    async fn copy_single_file(&self, src: &Path, dest: &Path, guest_dest: &str) -> Result<()> {
        // Sanity checks: src is in app_root...
        src.strip_prefix(&self.app_root)?;
        // ...and dest is in the Copy root.
        if let FilesMountStrategy::Copy(files_mount_root) = &self.files_mount_strategy {
            dest.strip_prefix(files_mount_root)?;
        } else {
            unreachable!();
        }

        let _loading_permit = self.file_loading_permits.acquire().await?;
        let dest_parent = parent_dir(dest)?;
        crate::fs::create_dir_all(&dest_parent)
            .await
            .with_context(|| {
                format!(
                    "Failed to create parent directory {}",
                    quoted_path(&dest_parent)
                )
            })?;
        crate::fs::copy(src, dest)
            .await
            .or_else(|e| Self::failed_to_copy_single_file_error(src, dest, guest_dest, e))?;
        tracing::debug!("Copied {src:?} to {dest:?}");
        Ok(())
    }

    fn failed_to_copy_single_file_error<T>(
        src: &Path,
        dest: &Path,
        guest_dest: &str,
        e: anyhow::Error,
    ) -> anyhow::Result<T> {
        let src_text = quoted_path(src);
        let dest_text = quoted_path(dest);
        let base_msg = format!("Failed to copy {src_text} to working path {dest_text}");

        if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
            if Self::is_directory_like(guest_dest)
                || io_error.kind() == std::io::ErrorKind::NotFound
            {
                return Err(anyhow::anyhow!(
                    r#""{guest_dest}" is not a valid destination file name"#
                ))
                .context(base_msg);
            }
        }

        Err(e).with_context(|| format!("{base_msg} (for destination path \"{guest_dest}\")"))
    }

    /// Does a guest path appear to be a directory name, e.g. "/" or ".."? This is for guest
    /// paths *only* and does not consider Windows separators.
    fn is_directory_like(guest_path: &str) -> bool {
        guest_path.ends_with('/') || guest_path.ends_with('.') || guest_path.ends_with("..")
    }

    // Resolve the given direct mount directory, checking that it is valid for
    // direct mounting and returning its canonicalized source path.
    async fn resolve_direct_mount(&self, mount: &WasiFilesMount) -> Result<ContentPath> {
        let (src, dest) = match mount {
            WasiFilesMount::Pattern(pattern) => (pattern, pattern),
            WasiFilesMount::Placement {
                source,
                destination,
            } => (source, destination),
        };
        let path = self.app_root.join(src);
        if !path.is_dir() {
            bail!("Only directory mounts are supported with `--direct-mounts`; {src:?} is not a directory.");
        }
        Ok(ContentPath {
            content: file_content_ref(src)?,
            path: dest.into(),
        })
    }
}

fn explain_file_mount_source_error(e: anyhow::Error, src: &Path) -> anyhow::Error {
    if let Some(io_error) = e.downcast_ref::<std::io::Error>() {
        if io_error.kind() == std::io::ErrorKind::NotFound {
            return anyhow::anyhow!("File or directory {} does not exist", quoted_path(src));
        }
    }
    e.context(format!("invalid file mount source {}", quoted_path(src)))
}

#[cfg(feature = "async-io")]
async fn verified_download(
    url: &str,
    digest: &str,
    dest: &Path,
    convention: crate::http::DestinationConvention,
) -> Result<()> {
    crate::http::verified_download(url, digest, dest, convention)
        .await
        .with_context(|| format!("Error fetching source URL {url:?}"))
}

#[cfg(not(feature = "async-io"))]
async fn verified_download(
    _url: &str,
    _digest: &str,
    _dest: &Path,
    _convention: crate::http::DestinationConvention,
) -> Result<()> {
    panic!("async-io feature is required for downloading Wasm sources")
}

fn safe_canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    use path_absolutize::Absolutize;
    Ok(path.absolutize()?.into_owned())
}

fn locked_metadata(
    details: v2::AppDetails,
    trigger_types: impl Iterator<Item = String>,
) -> Result<ValuesMap> {
    let mut builder = ValuesMapBuilder::new();
    builder
        .string("name", details.name)
        .string("version", details.version)
        .string("description", details.description)
        .string_array("authors", details.authors)
        .serializable("triggers", &details.trigger_global_configs)?;

    // Duplicate single-trigger global options into "trigger" with "type"
    // key to maintain backward compatibility for a while.
    let types = trigger_types.collect::<Vec<_>>();
    if types.len() == 1 {
        let trigger_type = types.into_iter().next().unwrap();
        let mut single_trigger = details
            .trigger_global_configs
            .get(&trigger_type)
            .cloned()
            .unwrap_or_default();
        single_trigger.insert("type".into(), trigger_type.into());
        builder.serializable("trigger", single_trigger).unwrap();
    }

    Ok(builder.build())
}

fn locked_variable(variable: v2::Variable) -> Result<locked::Variable> {
    ensure!(
        variable.required ^ variable.default.is_some(),
        "must be `required` OR have a `default`"
    );
    Ok(locked::Variable {
        default: variable.default.clone(),
        secret: variable.secret,
    })
}

fn locked_trigger(trigger_type: String, trigger: v2::Trigger) -> Result<LockedTrigger> {
    fn reference_id(spec: v2::ComponentSpec) -> toml::Value {
        let v2::ComponentSpec::Reference(id) = spec else {
            unreachable!("should have already been normalized");
        };
        id.as_ref().into()
    }

    let mut config = trigger.config;
    if let Some(id) = trigger.component.map(reference_id) {
        config.insert("component".into(), id);
    }
    if !trigger.components.is_empty() {
        // Flatten trigger config `components` `OneOrManyComponentSpecs` into
        // lists of component references.
        config.insert(
            "components".into(),
            trigger
                .components
                .into_iter()
                .map(|(key, specs)| {
                    (
                        key,
                        specs
                            .0
                            .into_iter()
                            .map(reference_id)
                            .collect::<Vec<_>>()
                            .into(),
                    )
                })
                .collect::<toml::Table>()
                .into(),
        );
    }

    Ok(LockedTrigger {
        id: trigger.id,
        trigger_type,
        trigger_config: config.try_into()?,
    })
}

#[derive(Debug)]
/// Handles loading of component Wasm from different sources.
pub struct WasmLoader {
    app_root: PathBuf,
    cache: Cache,
    file_loading_permits: std::sync::Arc<Semaphore>,
}

impl WasmLoader {
    /// Create a new instance of WasmLoader.
    pub async fn new(
        app_root: PathBuf,
        cache_root: Option<PathBuf>,
        file_loading_permits: Option<std::sync::Arc<Semaphore>>,
    ) -> Result<Self> {
        let file_loading_permits = file_loading_permits.unwrap_or_else(|| {
            std::sync::Arc::new(Semaphore::new(crate::MAX_FILE_LOADING_CONCURRENCY))
        });
        Ok(Self {
            app_root,
            cache: Cache::new(cache_root).await?,
            file_loading_permits,
        })
    }

    /// Load a Wasm source from the given ComponentSource and return a path
    /// to a file location from where it can be read.
    pub async fn load_component_source(
        &self,
        component_id: &str,
        source: &v2::ComponentSource,
    ) -> Result<PathBuf> {
        let content = match source {
            v2::ComponentSource::Local(path) => self.app_root.join(path),
            v2::ComponentSource::Remote { url, digest } => {
                self.load_http_source(url, digest).await?
            }
            v2::ComponentSource::Registry {
                registry,
                package,
                version,
            } => {
                let version = semver::Version::parse(version).with_context(|| format!("Component {component_id} specifies an invalid semantic version ({version:?}) for its package version"))?;
                let version_req = format!("={version}").parse().expect("version");

                self.load_registry_source(registry.as_ref(), package, &version_req)
                    .await?
            }
        };
        Ok(content)
    }

    // Load a Wasm source from the given HTTP ContentRef source URL and
    // return a ContentRef an absolute path to the local copy.
    async fn load_http_source(&self, url: &str, digest: &str) -> Result<PathBuf> {
        ensure!(
            digest.starts_with("sha256:"),
            "invalid `digest` {digest:?}; must start with 'sha256:'"
        );
        let path = if let Ok(cached_path) = self.cache.wasm_file(digest) {
            cached_path
        } else {
            let _loading_permit = self.file_loading_permits.acquire().await?;

            self.cache.ensure_dirs().await?;
            let dest = self.cache.wasm_path(digest);
            verified_download(
                url,
                digest,
                &dest,
                crate::http::DestinationConvention::ContentIndexed,
            )
            .await
            .with_context(|| format!("Error fetching source URL {url:?}"))?;
            dest
        };
        Ok(path)
    }

    async fn load_registry_source(
        &self,
        registry: Option<&wasm_pkg_client::Registry>,
        package: &wasm_pkg_client::PackageRef,
        version: &semver::VersionReq,
    ) -> Result<PathBuf> {
        let mut client_config = wasm_pkg_client::Config::global_defaults().await?;

        if let Some(registry) = registry.cloned() {
            let mapping = wasm_pkg_client::RegistryMapping::Registry(registry);
            client_config.set_package_registry_override(package.clone(), mapping);
        }
        let pkg_loader = wasm_pkg_client::Client::new(client_config);

        let mut releases = pkg_loader.list_all_versions(package).await.map_err(|e| {
            if matches!(e, wasm_pkg_client::Error::NoRegistryForNamespace(_)) && registry.is_none() {
                anyhow!("No default registry specified for wasm-pkg-loader. Create a default config, or set `registry` for package {package:?}")
            } else {
                e.into()
            }
        })?;

        releases.sort();

        let release_version = releases
            .iter()
            .rev()
            .find(|release| version.matches(&release.version) && !release.yanked)
            .with_context(|| format!("No matching version found for {package} {version}",))?;

        let release = pkg_loader
            .get_release(package, &release_version.version)
            .await?;

        let digest = match &release.content_digest {
            wasm_pkg_client::ContentDigest::Sha256 { hex } => format!("sha256:{hex}"),
        };

        let path = if let Ok(cached_path) = self.cache.wasm_file(&digest) {
            cached_path
        } else {
            let mut stm = pkg_loader.stream_content(package, &release).await?;

            self.cache.ensure_dirs().await?;
            let dest = self.cache.wasm_path(&digest);

            let mut file = tokio::fs::File::create(&dest).await?;
            while let Some(block) = stm.next().await {
                let bytes = block.context("Failed to get content from registry")?;
                file.write_all(&bytes)
                    .await
                    .context("Failed to save registry content to cache")?;
            }

            dest
        };

        Ok(path)
    }

    /// Loads a dependency
    pub async fn load_component_dependency(
        &self,
        dependency_name: &DependencyName,
        dependency: &v2::ComponentDependency,
    ) -> Result<(PathBuf, Option<String>)> {
        match dependency.clone() {
            v2::ComponentDependency::Version(version) => {
                let version = semver::VersionReq::parse(&version).with_context(|| format!("Component dependency {dependency_name:?} specifies an invalid semantic version requirement ({version:?}) for its package version"))?;

                // This `unwrap()` should be OK because we've already validated
                // this form of dependency requires a package name, i.e. the
                // dependency name is not a kebab id.
                let package = dependency_name.package().unwrap();

                let content = self.load_registry_source(None, package, &version).await?;
                Ok((content, None))
            }
            v2::ComponentDependency::Package {
                version,
                registry,
                package,
                export,
            } => {
                let version = semver::VersionReq::parse(&version).with_context(|| format!("Component dependency {dependency_name:?} specifies an invalid semantic version requirement ({version:?}) for its package version"))?;

                let package = match package {
                    Some(package) => {
                        package.parse().with_context(|| format!("Component dependency {dependency_name:?} specifies an invalid package name ({package:?})"))?
                    }
                    None => {
                        // This `unwrap()` should be OK because we've already validated
                        // this form of dependency requires a package name, i.e. the
                        // dependency name is not a kebab id.
                        dependency_name
                            .package()
                            .cloned()
                            .unwrap()
                    }
                };

                let registry = match registry {
                    Some(registry) => {
                        registry
                            .parse()
                            .map(Some)
                            .with_context(|| format!("Component dependency {dependency_name:?} specifies an invalid registry name ({registry:?})"))?
                    }
                    None => None,
                };

                let content = self
                    .load_registry_source(registry.as_ref(), &package, &version)
                    .await?;
                Ok((content, export))
            }
            v2::ComponentDependency::Local { path, export } => {
                let content = self.app_root.join(path);
                Ok((content, export))
            }
            v2::ComponentDependency::HTTP {
                url,
                digest,
                export,
            } => {
                let content = self.load_http_source(&url, &digest).await?;
                Ok((content, export))
            }
        }
    }
}

fn looks_like_glob_pattern(s: impl AsRef<str>) -> bool {
    let s = s.as_ref();
    glob::Pattern::escape(s) != s
}

fn file_content_ref(path: impl AsRef<Path>) -> Result<ContentRef> {
    Ok(ContentRef {
        source: Some(file_url(path)?),
        ..Default::default()
    })
}

fn file_url(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let abs_path = safe_canonicalize(path)
        .with_context(|| format!("Couldn't resolve `{}`", path.display()))?;
    Ok(Url::from_file_path(abs_path).unwrap().to_string())
}

fn requires_service_chaining(component: &spin_manifest::schema::v2::Component) -> bool {
    component
        .normalized_allowed_outbound_hosts()
        .unwrap_or_default()
        .iter()
        .any(|h| is_chaining_host(h))
}

fn is_chaining_host(pattern: &str) -> bool {
    use spin_outbound_networking_config::allowed_hosts::{AllowedHostConfig, HostConfig};

    let Ok(allowed) = AllowedHostConfig::parse(pattern) else {
        return false;
    };

    match allowed.host() {
        HostConfig::List(hosts) => hosts
            .iter()
            .any(|h| h.ends_with(SERVICE_CHAINING_DOMAIN_SUFFIX)),
        HostConfig::AnySubdomain(domain) => domain == SERVICE_CHAINING_DOMAIN_SUFFIX,
        _ => false,
    }
}

const SLOTH_WARNING_DELAY_MILLIS: u64 = 1250;

fn warn_if_component_load_slothful() -> sloth::SlothGuard {
    let message = "Loading Wasm components is taking a few seconds...";
    sloth::warn_if_slothful(SLOTH_WARNING_DELAY_MILLIS, format!("{message}\n"))
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn bad_destination_filename_is_explained() -> anyhow::Result<()> {
        let app_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("file-errors");
        let wd = tempfile::tempdir()?;
        let loader = LocalLoader::new(
            &app_root,
            FilesMountStrategy::Copy(wd.path().to_owned()),
            None,
        )
        .await?;
        let err = loader
            .load_file(app_root.join("bad.toml"))
            .await
            .expect_err("loader should not have succeeded");
        let err_ctx = format!("{err:#}");
        assert!(
            err_ctx.contains(r#""/" is not a valid destination file name"#),
            "expected error to show destination file name but got {}",
            err_ctx
        );
        Ok(())
    }
}
