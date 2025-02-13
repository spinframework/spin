//! Spin's client for distributing applications via OCI registries

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use docker_credential::DockerCredential;
use futures_util::future;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use itertools::Itertools;
use oci_distribution::{
    client::ImageLayer, config::ConfigFile, manifest::OciImageManifest, secrets::RegistryAuth,
    token_cache::RegistryTokenType, Reference, RegistryOperation,
};
use reqwest::Url;
use spin_common::sha256;
use spin_common::ui::quoted_path;
use spin_common::url::parse_file_url;
use spin_compose::ComponentSourceLoaderFs;
use spin_loader::cache::Cache;
use spin_loader::FilesMountStrategy;
use spin_locked_app::locked::{ContentPath, ContentRef, LockedApp, LockedComponent};
use tokio::fs;
use walkdir::WalkDir;

use crate::auth::AuthConfig;

// TODO: the media types for application, data and archive layer are not final
/// Media type for a layer representing a locked Spin application configuration
pub const SPIN_APPLICATION_MEDIA_TYPE: &str = "application/vnd.fermyon.spin.application.v1+config";
/// Media type for a layer representing a generic data file used by a Spin application
pub const DATA_MEDIATYPE: &str = "application/vnd.wasm.content.layer.v1+data";
/// Media type for a layer representing a compressed archive of one or more files used by a Spin application
pub const ARCHIVE_MEDIATYPE: &str = "application/vnd.wasm.content.bundle.v1.tar+gzip";
// Note: this will be updated with a canonical value once defined upstream
const WASM_LAYER_MEDIA_TYPE: &str = "application/vnd.wasm.content.layer.v1+wasm";

const CONFIG_FILE: &str = "config.json";
const LATEST_TAG: &str = "latest";
const MANIFEST_FILE: &str = "manifest.json";

/// Env var to force use of archive layers when publishing a Spin app
const SPIN_OCI_ARCHIVE_LAYERS_OPT: &str = "SPIN_OCI_ARCHIVE_LAYERS";

const MAX_PARALLEL_PULL: usize = 16;
/// Maximum layer count allowed per app, set in accordance to the lowest
/// known maximum per image in well-known OCI registry implementations.
/// (500 appears to be the limit for Elastic Container Registry)
const MAX_LAYER_COUNT: usize = 500;

/// Default maximum content size for inlining directly into config,
/// rather than pushing as a separate layer
const DEFAULT_CONTENT_REF_INLINE_MAX_SIZE: usize = 128;

/// Default token expiration when pushing/pulling an image to/from a registry.
/// This value is used by the underyling OCI client when the token expiration
/// is unspecified on a claim.
/// This essentially equates to a timeout for push/pull.
const DEFAULT_TOKEN_EXPIRATION_SECS: usize = 300;

/// Mode of assembly of a Spin application into an OCI image
#[derive(Copy, Clone)]
enum AssemblyMode {
    /// Assemble the application as one layer per component and one layer for
    /// every static asset included with a given component
    Simple,
    /// Assemble the application as one layer per component and one compressed
    /// archive layer containing all static assets included with a given component
    Archive,
}

/// Indicates whether to compose the components of a Spin application when pushing an image.
#[derive(Copy, Clone)]
pub enum ComposeMode {
    /// Compose components before pushing the image.
    All,
    /// Skip composing components before pushing the image.
    Skip,
}

/// Client for interacting with an OCI registry for Spin applications.
pub struct Client {
    /// Global cache for the metadata, Wasm modules, and static assets pulled from OCI registries.
    pub cache: Cache,
    /// Underlying OCI client.
    oci: oci_distribution::Client,
    /// Client options
    pub opts: ClientOpts,
}

#[derive(Clone)]
/// Options for configuring a Client
pub struct ClientOpts {
    /// Inline content into ContentRef iff < this size.
    pub content_ref_inline_max_size: usize,
}

/// Controls whether predefined annotations are generated when pushing an application.
/// If an explicit annotation has the same name as a predefined one, the explicit
/// one takes precedence.
#[derive(Debug, PartialEq)]
pub enum InferPredefinedAnnotations {
    /// Infer annotations for created, authors, version, name and description.
    All,
    /// Do not generate any annotations; use only explicitly supplied annotations.
    None,
}

impl Client {
    /// Create a new instance of an OCI client for distributing Spin applications.
    pub async fn new(insecure: bool, cache_root: Option<PathBuf>) -> Result<Self> {
        let client = oci_distribution::Client::new(Self::build_config(insecure));
        let cache = Cache::new(cache_root).await?;
        let opts = ClientOpts {
            content_ref_inline_max_size: DEFAULT_CONTENT_REF_INLINE_MAX_SIZE,
        };

        Ok(Self {
            oci: client,
            cache,
            opts,
        })
    }

    /// Push a Spin application to an OCI registry and return the digest (or None
    /// if the digest cannot be determined).
    pub async fn push(
        &mut self,
        manifest_path: &Path,
        reference: impl AsRef<str>,
        annotations: Option<BTreeMap<String, String>>,
        infer_annotations: InferPredefinedAnnotations,
        compose_mode: ComposeMode,
    ) -> Result<Option<String>> {
        let reference: Reference = reference
            .as_ref()
            .parse()
            .with_context(|| format!("cannot parse reference {}", reference.as_ref()))?;
        let auth = Self::auth(&reference).await?;
        let working_dir = tempfile::tempdir()?;

        // Create a locked application from the application manifest.
        // TODO: We don't need an extra copy here for each asset to prepare the application.
        // We should be able to use assets::collect instead when constructing the locked app.
        let locked = spin_loader::from_file(
            manifest_path,
            FilesMountStrategy::Copy(working_dir.path().into()),
            None,
        )
        .await?;

        self.push_locked_core(
            locked,
            auth,
            reference,
            annotations,
            infer_annotations,
            compose_mode,
        )
        .await
    }

    /// Push a Spin application to an OCI registry and return the digest (or None
    /// if the digest cannot be determined).
    pub async fn push_locked(
        &mut self,
        locked: LockedApp,
        reference: impl AsRef<str>,
        annotations: Option<BTreeMap<String, String>>,
        infer_annotations: InferPredefinedAnnotations,
        compose_mode: ComposeMode,
    ) -> Result<Option<String>> {
        let reference: Reference = reference
            .as_ref()
            .parse()
            .with_context(|| format!("cannot parse reference {}", reference.as_ref()))?;
        let auth = Self::auth(&reference).await?;

        self.push_locked_core(
            locked,
            auth,
            reference,
            annotations,
            infer_annotations,
            compose_mode,
        )
        .await
    }

    /// Push a Spin application to an OCI registry and return the digest (or None
    /// if the digest cannot be determined).
    async fn push_locked_core(
        &mut self,
        locked: LockedApp,
        auth: RegistryAuth,
        reference: Reference,
        annotations: Option<BTreeMap<String, String>>,
        infer_annotations: InferPredefinedAnnotations,
        compose_mode: ComposeMode,
    ) -> Result<Option<String>> {
        let mut locked_app = locked.clone();
        let mut layers = self
            .assemble_layers(&mut locked_app, AssemblyMode::Simple, compose_mode)
            .await
            .context("could not assemble layers for locked application")?;

        // If SPIN_OCI_ARCHIVE_LAYERS_OPT is set *or* if layer count exceeds MAX_LAYER_COUNT-1,
        // assemble archive layers instead. (An additional layer to represent the locked
        // application config is added.)
        if std::env::var(SPIN_OCI_ARCHIVE_LAYERS_OPT).is_ok() || layers.len() > MAX_LAYER_COUNT - 1
        {
            locked_app = locked.clone();
            layers = self
                .assemble_layers(&mut locked_app, AssemblyMode::Archive, compose_mode)
                .await
                .context("could not assemble archive layers for locked application")?;
        }

        let annotations = all_annotations(&locked_app, annotations, infer_annotations);

        // Push layer for locked spin application config
        let locked_config_layer = ImageLayer::new(
            serde_json::to_vec(&locked_app).context("could not serialize locked config")?,
            SPIN_APPLICATION_MEDIA_TYPE.to_string(),
            None,
        );
        let config_layer_digest = locked_config_layer.sha256_digest().clone();
        layers.push(locked_config_layer);

        let mut labels = HashMap::new();
        labels.insert(
            "com.fermyon.spin.lockedAppDigest".to_string(),
            config_layer_digest,
        );
        let cfg = oci_distribution::config::Config {
            labels: Some(labels),
            ..Default::default()
        };

        // Construct empty/default OCI config file. Data may be parsed according to
        // the expected config structure per the image spec, so we want to ensure it conforms.
        // (See https://github.com/opencontainers/image-spec/blob/main/config.md)
        // TODO: Explore adding data applicable to the Spin app being published.
        let oci_config_file = ConfigFile {
            architecture: oci_distribution::config::Architecture::Wasm,
            os: oci_distribution::config::Os::Wasip1,
            // We need to ensure that the image config for different content is updated.
            // Without referencing the digest of the locked application in the OCI image config,
            // all Spin applications would get the same image config digest, resulting in the same
            // image ID in container runtimes.
            config: Some(cfg),
            ..Default::default()
        };
        let oci_config =
            oci_distribution::client::Config::oci_v1_from_config_file(oci_config_file, None)?;
        let manifest = OciImageManifest::build(&layers, &oci_config, annotations);

        let response = self
            .oci
            .push(&reference, &layers, oci_config, &auth, Some(manifest))
            .await
            .map(|push_response| push_response.manifest_url)
            .context("cannot push Spin application")?;

        tracing::info!("Pushed {:?}", response);

        let digest = digest_from_url(&response);
        Ok(digest)
    }

    /// Assemble ImageLayers for a locked application using the provided
    /// AssemblyMode and return the resulting Vec<ImageLayer>.
    async fn assemble_layers(
        &mut self,
        locked: &mut LockedApp,
        assembly_mode: AssemblyMode,
        compose_mode: ComposeMode,
    ) -> Result<Vec<ImageLayer>> {
        let (mut layers, components) = match compose_mode {
            ComposeMode::All => {
                self.assemble_layers_composed(assembly_mode, locked.clone())
                    .await?
            }
            ComposeMode::Skip => {
                self.assemble_layers_uncomposed(assembly_mode, locked.clone())
                    .await?
            }
        };

        locked.components = components;
        locked.metadata.remove("origin");

        // Deduplicate layers
        layers = layers.into_iter().unique().collect();

        Ok(layers)
    }

    async fn assemble_layers_uncomposed(
        &mut self,
        assembly_mode: AssemblyMode,
        locked: LockedApp,
    ) -> Result<(Vec<ImageLayer>, Vec<LockedComponent>)> {
        let mut components = Vec::new();
        let mut layers = Vec::new();

        for mut c in locked.components {
            // Add the wasm module for the component as layers.
            let source = c
                .source
                .content
                .source
                .as_ref()
                .context("component loaded from disk should contain a file source")?;

            let source = parse_file_url(source.as_str())?;
            let layer = Self::wasm_layer(&source).await?;

            // Update the module source with the content ref of the layer.
            c.source.content = self.content_ref_for_layer(&layer);

            layers.push(layer);

            let mut deps = BTreeMap::default();
            for (dep_name, mut dep) in c.dependencies {
                let source = dep
                    .source
                    .content
                    .source
                    .context("dependency loaded from disk should contain a file source")?;
                let source = parse_file_url(source.as_str())?;

                let layer = Self::wasm_layer(&source).await?;

                dep.source.content = self.content_ref_for_layer(&layer);
                deps.insert(dep_name, dep);

                layers.push(layer);
            }
            c.dependencies = deps;

            c.files = self
                .assemble_content_layers(assembly_mode, &mut layers, c.files.as_slice())
                .await?;
            components.push(c);
        }

        Ok((layers, components))
    }

    async fn assemble_layers_composed(
        &mut self,
        assembly_mode: AssemblyMode,
        locked: LockedApp,
    ) -> Result<(Vec<ImageLayer>, Vec<LockedComponent>)> {
        let mut components = Vec::new();
        let mut layers = Vec::new();

        for mut c in locked.components {
            let composed = spin_compose::compose(&ComponentSourceLoaderFs, &c)
                .await
                .with_context(|| {
                    format!("failed to resolve dependencies for component {:?}", c.id)
                })?;
            let layer = ImageLayer::new(composed, WASM_LAYER_MEDIA_TYPE.to_string(), None);
            c.source.content = self.content_ref_for_layer(&layer);
            c.dependencies.clear();
            layers.push(layer);

            c.files = self
                .assemble_content_layers(assembly_mode, &mut layers, c.files.as_slice())
                .await?;
            components.push(c);
        }

        Ok((layers, components))
    }

    async fn assemble_content_layers(
        &mut self,
        assembly_mode: AssemblyMode,
        layers: &mut Vec<ImageLayer>,
        contents: &[ContentPath],
    ) -> Result<Vec<ContentPath>> {
        let mut files = Vec::new();
        for f in contents {
            let source = f
                .content
                .source
                .as_ref()
                .context("file mount loaded from disk should contain a file source")?;
            let source = parse_file_url(source.as_str())?;

            match assembly_mode {
                AssemblyMode::Archive => self
                    .push_archive_layer(&source, &mut files, layers)
                    .await
                    .context(format!(
                        "cannot push archive layer for source {}",
                        quoted_path(&source)
                    ))?,
                AssemblyMode::Simple => self
                    .push_file_layers(&source, &mut files, layers)
                    .await
                    .context(format!(
                        "cannot push file layers for source {}",
                        quoted_path(&source)
                    ))?,
            }
        }
        Ok(files)
    }

    /// Archive all of the files recursively under the source directory
    /// and push as a compressed archive layer
    async fn push_archive_layer(
        &mut self,
        source: &PathBuf,
        files: &mut Vec<ContentPath>,
        layers: &mut Vec<ImageLayer>,
    ) -> Result<()> {
        // Add all archived file entries to the locked app manifest
        for entry in WalkDir::new(source) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            // Can unwrap because we got to 'entry' from walking 'source'
            let rel_path = entry.path().strip_prefix(source).unwrap();
            tracing::trace!("Adding asset {rel_path:?} to component files list");
            // Add content/path to the locked component files list
            let layer = Self::data_layer(entry.path(), DATA_MEDIATYPE.to_string()).await?;
            let content = self.content_ref_for_layer(&layer);
            files.push(ContentPath {
                content,
                path: rel_path.into(),
            });
        }

        // Only add the archive layer to the OCI manifest
        tracing::trace!("Adding archive layer for all files in source {:?}", &source);
        let working_dir = tempfile::tempdir()?;
        let archive_path = crate::utils::archive(source, &working_dir.into_path())
            .await
            .context(format!(
                "Unable to create compressed archive for source {:?}",
                source
            ))?;
        let layer = Self::data_layer(archive_path.as_path(), ARCHIVE_MEDIATYPE.to_string()).await?;
        layers.push(layer);
        Ok(())
    }

    /// Recursively traverse the source directory and add layers for each file.
    async fn push_file_layers(
        &mut self,
        source: &PathBuf,
        files: &mut Vec<ContentPath>,
        layers: &mut Vec<ImageLayer>,
    ) -> Result<()> {
        // Traverse each mount directory, add all static assets as layers, then update the
        // locked application file with the file digest.
        tracing::trace!("Adding new layer per file under source {:?}", source);
        for entry in WalkDir::new(source) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            // Can unwrap because we got to 'entry' from walking 'source'
            let rel_path = entry.path().strip_prefix(source).unwrap();
            // Paths must be in portable (forward slash) format in the registry,
            // so that they can be placed correctly on any host system
            let rel_path = portable_path(rel_path);

            tracing::trace!("Adding new layer for asset {rel_path:?}");
            // Construct and push layer, adding its digest to the locked component files Vec
            let layer = Self::data_layer(entry.path(), DATA_MEDIATYPE.to_string()).await?;
            let content = self.content_ref_for_layer(&layer);
            let content_inline = content.inline.is_some();
            files.push(ContentPath {
                content,
                path: rel_path,
            });
            // As a workaround for OCI implementations that don't support very small blobs,
            // don't push very small content that has been inlined into the manifest:
            // https://github.com/distribution/distribution/discussions/4029
            let skip_layer = content_inline;
            if !skip_layer {
                layers.push(layer);
            }
        }
        Ok(())
    }

    /// Pull a Spin application from an OCI registry.
    pub async fn pull(&mut self, reference: &str) -> Result<()> {
        let reference: Reference = reference.parse().context("cannot parse reference")?;
        let auth = Self::auth(&reference).await?;

        // Pull the manifest from the registry.
        let (manifest, digest) = self.oci.pull_image_manifest(&reference, &auth).await?;

        let manifest_json = serde_json::to_string(&manifest)?;
        tracing::debug!("Pulled manifest: {}", manifest_json);

        // Write the manifest in `<cache_root>/registry/oci/manifests/repository:<tag_or_latest>/manifest.json`
        let m = self.manifest_path(&reference.to_string()).await?;
        fs::write(&m, &manifest_json).await?;

        // Older published Spin apps feature the locked app config *as* the OCI manifest config layer,
        // while newer versions publish the locked app config as a generic layer alongside others.
        // Assume that these bytes may represent the locked app config and write it as such.
        let mut cfg_bytes = Vec::new();
        self.oci
            .pull_blob(&reference, &manifest.config, &mut cfg_bytes)
            .await?;
        self.write_locked_app_config(&reference.to_string(), &cfg_bytes)
            .await
            .context("unable to write locked app config to cache")?;

        // If a layer is a Wasm module, write it in the Wasm directory.
        // Otherwise, write it in the data directory (after unpacking if archive layer)
        stream::iter(manifest.layers)
            .map(|layer| {
                let this = &self;
                let reference = reference.clone();
                async move {
                    // Skip pulling if the digest already exists in the wasm or data directories.
                    if this.cache.wasm_file(&layer.digest).is_ok()
                        || this.cache.data_file(&layer.digest).is_ok()
                    {
                        tracing::debug!("Layer {} already exists in cache", &layer.digest);
                        return anyhow::Ok(());
                    }

                    tracing::debug!("Pulling layer {}", &layer.digest);
                    let mut bytes = Vec::with_capacity(layer.size.try_into()?);
                    this.oci.pull_blob(&reference, &layer, &mut bytes).await?;
                    match layer.media_type.as_str() {
                        SPIN_APPLICATION_MEDIA_TYPE => {
                            this.write_locked_app_config(&reference.to_string(), &bytes)
                                .await
                                .with_context(|| "unable to write locked app config to cache")?;
                        }
                        WASM_LAYER_MEDIA_TYPE => {
                            this.cache.write_wasm(&bytes, &layer.digest).await?;
                        }
                        ARCHIVE_MEDIATYPE => {
                            unpack_archive_layer(&this.cache, &bytes, &layer.digest).await?;
                        }
                        _ => {
                            this.cache.write_data(&bytes, &layer.digest).await?;
                        }
                    }
                    Ok(())
                }
            })
            .buffer_unordered(MAX_PARALLEL_PULL)
            .try_for_each(future::ok)
            .await?;
        tracing::info!("Pulled {}@{}", reference, digest);

        Ok(())
    }

    /// Get the file path to an OCI manifest given a reference.
    /// If the directory for the manifest does not exist, this will create it.
    async fn manifest_path(&self, reference: impl AsRef<str>) -> Result<PathBuf> {
        let reference: Reference = reference
            .as_ref()
            .parse()
            .context("cannot parse OCI reference")?;
        let p = self
            .cache
            .manifests_dir()
            .join(fs_safe_segment(reference.registry()))
            .join(reference.repository())
            .join(reference.tag().unwrap_or(LATEST_TAG));

        if !p.is_dir() {
            fs::create_dir_all(&p).await.with_context(|| {
                format!("cannot create directory {} for OCI manifest", p.display())
            })?;
        }

        Ok(p.join(MANIFEST_FILE))
    }

    /// Get the file path to the OCI configuration object given a reference.
    pub async fn lockfile_path(&self, reference: impl AsRef<str>) -> Result<PathBuf> {
        let reference: Reference = reference
            .as_ref()
            .parse()
            .context("cannot parse reference")?;
        let p = self
            .cache
            .manifests_dir()
            .join(fs_safe_segment(reference.registry()))
            .join(reference.repository())
            .join(reference.tag().unwrap_or(LATEST_TAG));

        if !p.is_dir() {
            fs::create_dir_all(&p)
                .await
                .context("cannot find configuration object for reference")?;
        }

        Ok(p.join(CONFIG_FILE))
    }

    /// Write the config object in `<cache_root>/registry/oci/manifests/repository:<tag_or_latest>/config.json`
    async fn write_locked_app_config(
        &self,
        reference: impl AsRef<str>,
        bytes: impl AsRef<[u8]>,
    ) -> Result<()> {
        let cfg = std::str::from_utf8(bytes.as_ref())?;
        tracing::debug!("Pulled config: {}", cfg);

        let c = self.lockfile_path(reference).await?;
        fs::write(&c, &cfg).await.map_err(anyhow::Error::from)
    }

    /// Create a new wasm layer based on a file.
    async fn wasm_layer(file: &Path) -> Result<ImageLayer> {
        tracing::trace!("Reading wasm module from {:?}", file);
        Ok(ImageLayer::new(
            fs::read(file)
                .await
                .with_context(|| format!("cannot read wasm module {}", quoted_path(file)))?,
            WASM_LAYER_MEDIA_TYPE.to_string(),
            None,
        ))
    }

    /// Create a new data layer based on a file.
    async fn data_layer(file: &Path, media_type: String) -> Result<ImageLayer> {
        tracing::trace!("Reading data file from {:?}", file);
        Ok(ImageLayer::new(
            fs::read(&file)
                .await
                .with_context(|| format!("cannot read file {}", quoted_path(file)))?,
            media_type,
            None,
        ))
    }

    fn content_ref_for_layer(&self, layer: &ImageLayer) -> ContentRef {
        ContentRef {
            // Inline small content as an optimization and to work around issues
            // with OCI implementations that don't support very small blobs.
            inline: (layer.data.len() <= self.opts.content_ref_inline_max_size)
                .then(|| layer.data.to_vec()),
            digest: Some(layer.sha256_digest()),
            ..Default::default()
        }
    }

    /// Save a credential set containing the registry username and password.
    pub async fn login(
        server: impl AsRef<str>,
        username: impl AsRef<str>,
        password: impl AsRef<str>,
    ) -> Result<()> {
        let registry = registry_from_input(server);

        // First, validate the credentials. If a user accidentally enters a wrong credential set, this
        // can catch the issue early rather than getting an error at the first operation that needs
        // to use the credentials (first time they do a push/pull/up).
        Self::validate_credentials(&registry, &username, &password).await?;

        // Save an encoded representation of the credential set in the local configuration file.
        let mut auth = AuthConfig::load_default().await?;
        auth.insert(registry, username, password)?;
        auth.save_default().await
    }

    /// Insert a token in the OCI client token cache.
    pub async fn insert_token(
        &mut self,
        reference: &Reference,
        op: RegistryOperation,
        token: RegistryTokenType,
    ) {
        self.oci.tokens.insert(reference, op, token).await;
    }

    /// Validate the credentials by attempting to send an authenticated request to the registry.
    async fn validate_credentials(
        server: impl AsRef<str>,
        username: impl AsRef<str>,
        password: impl AsRef<str>,
    ) -> Result<()> {
        let client = dkregistry::v2::Client::configure()
            .registry(server.as_ref())
            .insecure_registry(false)
            .username(Some(username.as_ref().into()))
            .password(Some(password.as_ref().into()))
            .build()
            .context("cannot create client to send authentication request to the registry")?;

        match client
            // We don't need to configure any scopes, we are only testing that the credentials are
            // valid for the intended registry.
            .authenticate(&[""])
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => bail!(format!(
                "cannot authenticate as {} to registry {}: {}",
                username.as_ref(),
                server.as_ref(),
                e
            )),
        }
    }

    /// Construct the registry authentication based on the reference.
    async fn auth(reference: &Reference) -> Result<RegistryAuth> {
        let server = reference
            .resolve_registry()
            .strip_suffix('/')
            .unwrap_or_else(|| reference.resolve_registry());

        match AuthConfig::get_auth_from_default(server).await {
            Ok(c) => Ok(c),
            Err(_) => match docker_credential::get_credential(server) {
                Err(e) => {
                    tracing::trace!("Cannot retrieve credentials from Docker, attempting to use anonymous auth: {}", e);
                    Ok(RegistryAuth::Anonymous)
                }

                Ok(DockerCredential::UsernamePassword(username, password)) => {
                    tracing::trace!("Found Docker credentials");
                    Ok(RegistryAuth::Basic(username, password))
                }
                Ok(DockerCredential::IdentityToken(_)) => {
                    tracing::trace!("Cannot use contents of Docker config, identity token not supported. Using anonymous auth");
                    Ok(RegistryAuth::Anonymous)
                }
            },
        }
    }

    /// Build the OCI client configuration given the insecure option.
    fn build_config(insecure: bool) -> oci_distribution::client::ClientConfig {
        let protocol = if insecure {
            oci_distribution::client::ClientProtocol::Http
        } else {
            oci_distribution::client::ClientProtocol::Https
        };

        oci_distribution::client::ClientConfig {
            protocol,
            default_token_expiration_secs: DEFAULT_TOKEN_EXPIRATION_SECS,
            ..Default::default()
        }
    }
}

/// Unpack contents of the provided archive layer, represented by bytes and its
/// corresponding digest, into the provided cache.
/// A temporary staging directory is created via tempfile::tempdir() to store
/// the unpacked contents prior to writing to the cache.
pub async fn unpack_archive_layer(
    cache: &Cache,
    bytes: impl AsRef<[u8]>,
    digest: impl AsRef<str>,
) -> Result<()> {
    // Write archive layer to cache as usual
    cache.write_data(&bytes, &digest).await?;

    // Unpack archive into a staging dir
    let path = cache
        .data_file(&digest)
        .context("unable to read archive layer from cache")?;
    let staging_dir = tempfile::tempdir()?;
    crate::utils::unarchive(path.as_ref(), staging_dir.path()).await?;

    // Traverse unpacked contents and if a file, write to cache by digest
    // (if it doesn't already exist)
    for entry in WalkDir::new(staging_dir.path()) {
        let entry = entry?;
        if entry.file_type().is_file() && !entry.file_type().is_dir() {
            let bytes = tokio::fs::read(entry.path()).await?;
            let digest = format!("sha256:{}", sha256::hex_digest_from_bytes(&bytes));
            if cache.data_file(&digest).is_ok() {
                tracing::debug!(
                    "Skipping unpacked asset {:?}; file already exists",
                    entry.path()
                );
            } else {
                tracing::debug!("Adding unpacked asset {:?} to cache", entry.path());
                cache.write_data(bytes, &digest).await?;
            }
        }
    }
    Ok(())
}

fn digest_from_url(manifest_url: &str) -> Option<String> {
    // The URL is in the form "https://host/v2/refname/manifests/sha256:..."
    let manifest_url = Url::parse(manifest_url).ok()?;
    let segments = manifest_url.path_segments()?;
    let last = segments.last()?;
    if last.contains(':') {
        Some(last.to_owned())
    } else {
        None
    }
}

fn registry_from_input(server: impl AsRef<str>) -> String {
    // We want to allow a user to login to both https://ghcr.io and ghcr.io.
    let server = server.as_ref();
    let server = match server.parse::<Url>() {
        Ok(url) => url.host_str().unwrap_or(server).to_string(),
        Err(_) => server.to_string(),
    };
    // DockerHub is commonly referenced as 'docker.io' but needs to be 'index.docker.io'
    match server.as_str() {
        "docker.io" => "index.docker.io".to_string(),
        _ => server,
    }
}

fn all_annotations(
    locked_app: &LockedApp,
    explicit: Option<BTreeMap<String, String>>,
    predefined: InferPredefinedAnnotations,
) -> Option<BTreeMap<String, String>> {
    use spin_locked_app::{MetadataKey, APP_DESCRIPTION_KEY, APP_NAME_KEY, APP_VERSION_KEY};
    const APP_AUTHORS_KEY: MetadataKey<Vec<String>> = MetadataKey::new("authors");

    if predefined == InferPredefinedAnnotations::None {
        return explicit;
    }

    // We will always, at minimum, have a `created` annotation, so if we don't already have an
    // anootations collection then we may as well create one now...
    let mut current = explicit.unwrap_or_default();

    let authors = locked_app
        .get_metadata(APP_AUTHORS_KEY)
        .unwrap_or_default()
        .unwrap_or_default();
    if !authors.is_empty() {
        let authors = authors.join(", ");
        add_inferred(
            &mut current,
            oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_AUTHORS,
            Some(authors),
        );
    }

    let name = locked_app.get_metadata(APP_NAME_KEY).unwrap_or_default();
    add_inferred(
        &mut current,
        oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_TITLE,
        name,
    );

    let description = locked_app
        .get_metadata(APP_DESCRIPTION_KEY)
        .unwrap_or_default();
    add_inferred(
        &mut current,
        oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_DESCRIPTION,
        description,
    );

    let version = locked_app.get_metadata(APP_VERSION_KEY).unwrap_or_default();
    add_inferred(
        &mut current,
        oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_VERSION,
        version,
    );

    let created = chrono::Utc::now().to_rfc3339();
    add_inferred(
        &mut current,
        oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_CREATED,
        Some(created),
    );

    Some(current)
}

fn add_inferred(map: &mut BTreeMap<String, String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        if let std::collections::btree_map::Entry::Vacant(e) = map.entry(key.to_string()) {
            e.insert(value);
        }
    }
}

/// Takes a relative path and turns it into a format that is safe
/// for putting into a registry where it might end up on any host.
#[cfg(target_os = "windows")]
fn portable_path(rel_path: &Path) -> PathBuf {
    assert!(
        rel_path.is_relative(),
        "portable_path requires paths to be relative"
    );
    let portable_path = rel_path.to_string_lossy().replace('\\', "/");
    PathBuf::from(portable_path)
}

/// Takes a relative path and turns it into a format that is safe
/// for putting into a registry where it might end up on any host.
/// This is a no-op on Unix systems, but is needed for Windows.
#[cfg(not(target_os = "windows"))]
fn portable_path(rel_path: &Path) -> PathBuf {
    rel_path.into()
}

/// Takes a string intended for use as part of a path and makes it
/// compatible with the local filesystem.
#[cfg(target_os = "windows")]
fn fs_safe_segment(segment: &str) -> impl AsRef<Path> {
    segment.replace(':', "_")
}

/// Takes a string intended for use as part of a path and makes it
/// compatible with the local filesystem.
/// This is a no-op on Unix systems, but is needed for Windows.
#[cfg(not(target_os = "windows"))]
fn fs_safe_segment(segment: &str) -> impl AsRef<Path> + '_ {
    segment
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_parse_digest_from_manifest_url() {
        let manifest_url = "https://ghcr.io/v2/itowlson/osf/manifests/sha256:0a867093096e0ef01ef749b12b6e7a90e4952eda107f89a676eeedce63a8361f";
        let digest = digest_from_url(manifest_url).unwrap();
        assert_eq!(
            "sha256:0a867093096e0ef01ef749b12b6e7a90e4952eda107f89a676eeedce63a8361f",
            digest
        );
    }

    #[test]
    fn can_derive_registry_from_input() {
        #[derive(Clone)]
        struct TestCase {
            input: &'static str,
            want: &'static str,
        }
        let tests: Vec<TestCase> = [
            TestCase {
                input: "docker.io",
                want: "index.docker.io",
            },
            TestCase {
                input: "index.docker.io",
                want: "index.docker.io",
            },
            TestCase {
                input: "https://ghcr.io",
                want: "ghcr.io",
            },
        ]
        .to_vec();

        for tc in tests {
            assert_eq!(tc.want, registry_from_input(tc.input));
        }
    }

    // Convenience wrapper for deserializing from literal JSON
    #[macro_export]
    #[allow(missing_docs)] // it's test-only, but rust-analyzer gets mad
    macro_rules! from_json {
        ($($json:tt)+) => {
            serde_json::from_value(serde_json::json!($($json)+)).expect("valid json")
        };
    }

    #[tokio::test]
    async fn can_assemble_layers() {
        use spin_locked_app::locked::LockedComponent;
        use tokio::io::AsyncWriteExt;

        let working_dir = tempfile::tempdir().unwrap();

        // Set up component/file directory tree
        //
        // create component1 and component2 dirs
        let _ = tokio::fs::create_dir(working_dir.path().join("component1").as_path()).await;
        let _ = tokio::fs::create_dir(working_dir.path().join("component2").as_path()).await;

        // create component "wasm" files
        let mut c1 = tokio::fs::File::create(working_dir.path().join("component1.wasm"))
            .await
            .expect("should create component wasm file");
        c1.write_all(b"c1")
            .await
            .expect("should write component wasm contents");
        let mut c2 = tokio::fs::File::create(working_dir.path().join("component2.wasm"))
            .await
            .expect("should create component wasm file");
        c2.write_all(b"c2")
            .await
            .expect("should write component wasm contents");

        // component1 files
        let mut c1f1 = tokio::fs::File::create(working_dir.path().join("component1").join("bar"))
            .await
            .expect("should create component file");
        c1f1.write_all(b"bar")
            .await
            .expect("should write file contents");
        let mut c1f2 = tokio::fs::File::create(working_dir.path().join("component1").join("baz"))
            .await
            .expect("should create component file");
        c1f2.write_all(b"baz")
            .await
            .expect("should write file contents");

        // component2 files
        let mut c2f1 = tokio::fs::File::create(working_dir.path().join("component2").join("baz"))
            .await
            .expect("should create component file");
        c2f1.write_all(b"baz")
            .await
            .expect("should write file contents");

        // create a component with dependencies
        const TEST_WIT: &str = "
        package test:test;

        interface a {
            a: func();
        }

        world dep-a {
            export a;
        }

        world root {
            import a;
        }
        ";

        let root = generate_dummy_component(TEST_WIT, "root");
        let dep_a = generate_dummy_component(TEST_WIT, "dep-a");

        let mut r = tokio::fs::File::create(working_dir.path().join("root.wasm"))
            .await
            .expect("should create component wasm file");
        r.write_all(&root)
            .await
            .expect("should write component wasm contents");

        let mut a = tokio::fs::File::create(working_dir.path().join("dep_a.wasm"))
            .await
            .expect("should create component wasm file");
        a.write_all(&dep_a)
            .await
            .expect("should write component wasm contents");

        #[derive(Clone)]
        struct TestCase {
            name: &'static str,
            opts: Option<ClientOpts>,
            locked_components: Vec<LockedComponent>,
            expected_layer_count: usize,
            expected_error: Option<&'static str>,
            compose_mode: ComposeMode,
        }

        let tests: Vec<TestCase> = [
            TestCase {
                name: "Two component layers",
                opts: None,
                locked_components: from_json!([{
                    "id": "component1",
                    "source": {
                        "content_type": "application/wasm",
                        "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                        "digest": "digest",
                }},
                {
                    "id": "component2",
                    "source": {
                        "content_type": "application/wasm",
                        "source": format!("file://{}", working_dir.path().join("component2.wasm").to_str().unwrap()),
                        "digest": "digest",
                }}]),
                expected_layer_count: 2,
                expected_error: None,
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "One component layer and two file layers",
                opts: Some(ClientOpts{content_ref_inline_max_size: 0}),
                locked_components: from_json!([{
                "id": "component1",
                "source": {
                    "content_type": "application/wasm",
                    "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                    "digest": "digest",
                },
                "files": [
                    {
                        "source": format!("file://{}", working_dir.path().join("component1").to_str().unwrap()),
                        "path": working_dir.path().join("component1").join("bar").to_str().unwrap()
                    },
                    {
                        "source": format!("file://{}", working_dir.path().join("component1").to_str().unwrap()),
                        "path": working_dir.path().join("component1").join("baz").to_str().unwrap()
                    }
                ]
                }]),
                expected_layer_count: 3,
                expected_error: None,
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "One component layer and one file with inlined content",
                opts: None,
                locked_components: from_json!([{
                "id": "component1",
                "source": {
                    "content_type": "application/wasm",
                    "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                    "digest": "digest",
                },
                "files": [
                    {
                        "source": format!("file://{}", working_dir.path().join("component1").to_str().unwrap()),
                        "path": working_dir.path().join("component1").join("bar").to_str().unwrap()
                    }
                ]
                }]),
                expected_layer_count: 1,
                expected_error: None,
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "One component layer and one dependency component layer skipping composition",
                opts: Some(ClientOpts{content_ref_inline_max_size: 0}),
                locked_components: from_json!([{
                "id": "component1",
                "source": {
                    "content_type": "application/wasm",
                    "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                    "digest": "digest",
                },
                "dependencies": {
                    "test:comp2": {
                        "source": {
                            "content_type": "application/wasm",
                            "source": format!("file://{}", working_dir.path().join("component2.wasm").to_str().unwrap()),
                            "digest": "digest",
                        },
                        "export": null,
                    }
                }
                }]),
                expected_layer_count: 2,
                expected_error: None,
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "Component has no source",
                opts: None,
                locked_components: from_json!([{
                "id": "component1",
                "source": {
                    "content_type": "application/wasm",
                    "source": "",
                    "digest": "digest",
                }
                }]),
                expected_layer_count: 0,
                expected_error: Some("Invalid URL: \"\""),
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "Duplicate component sources",
                opts: None,
                locked_components: from_json!([{
                    "id": "component1",
                    "source": {
                        "content_type": "application/wasm",
                        "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                        "digest": "digest",
                }},
                {
                    "id": "component2",
                    "source": {
                        "content_type": "application/wasm",
                        "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                        "digest": "digest",
                }}]),
                expected_layer_count: 1,
                expected_error: None,
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "Duplicate file paths",
                opts: Some(ClientOpts{content_ref_inline_max_size: 0}),
                locked_components: from_json!([{
                "id": "component1",
                "source": {
                    "content_type": "application/wasm",
                    "source": format!("file://{}", working_dir.path().join("component1.wasm").to_str().unwrap()),
                    "digest": "digest",
                },
                "files": [
                    {
                        "source": format!("file://{}", working_dir.path().join("component1").to_str().unwrap()),
                        "path": working_dir.path().join("component1").join("bar").to_str().unwrap()
                    },
                    {
                        "source": format!("file://{}", working_dir.path().join("component1").to_str().unwrap()),
                        "path": working_dir.path().join("component1").join("baz").to_str().unwrap()
                    }
                ]},
                {
                    "id": "component2",
                    "source": {
                        "content_type": "application/wasm",
                        "source": format!("file://{}", working_dir.path().join("component2.wasm").to_str().unwrap()),
                        "digest": "digest",
                },
                "files": [
                    {
                        "source": format!("file://{}", working_dir.path().join("component2").to_str().unwrap()),
                        "path": working_dir.path().join("component2").join("baz").to_str().unwrap()
                    }
                ]
                }]),
                expected_layer_count: 4,
                expected_error: None,
                compose_mode: ComposeMode::Skip,
            },
            TestCase {
                name: "One component layer and one dependency component layer with composition",
                opts: Some(ClientOpts{content_ref_inline_max_size: 0}),
                locked_components: from_json!([{
                "id": "component-with-deps",
                "source": {
                    "content_type": "application/wasm",
                    "source": format!("file://{}", working_dir.path().join("root.wasm").to_str().unwrap()),
                    "digest": "digest",
                },
                "dependencies": {
                    "test:test/a": {
                        "source": {
                            "content_type": "application/wasm",
                            "source": format!("file://{}", working_dir.path().join("dep_a.wasm").to_str().unwrap()),
                            "digest": "digest",
                        },
                        "export": null,
                    }
                }
                }]),
                expected_layer_count: 1,
                expected_error: None,
                compose_mode: ComposeMode::All,
            },
        ]
        .to_vec();

        for tc in tests {
            let triggers = Default::default();
            let metadata = Default::default();
            let variables = Default::default();
            let mut locked = LockedApp {
                spin_lock_version: Default::default(),
                components: tc.locked_components,
                triggers,
                metadata,
                variables,
                must_understand: Default::default(),
                host_requirements: Default::default(),
            };

            let mut client = Client::new(false, Some(working_dir.path().to_path_buf()))
                .await
                .expect("should create new client");
            if let Some(o) = tc.opts {
                client.opts = o;
            }

            match tc.expected_error {
                Some(e) => {
                    assert_eq!(
                        e,
                        client
                            .assemble_layers(&mut locked, AssemblyMode::Simple, tc.compose_mode)
                            .await
                            .unwrap_err()
                            .to_string(),
                        "{}",
                        tc.name
                    )
                }
                None => {
                    assert_eq!(
                        tc.expected_layer_count,
                        client
                            .assemble_layers(&mut locked, AssemblyMode::Simple, tc.compose_mode)
                            .await
                            .unwrap()
                            .len(),
                        "{}",
                        tc.name
                    )
                }
            }
        }
    }

    fn generate_dummy_component(wit: &str, world: &str) -> Vec<u8> {
        let mut resolve = wit_parser::Resolve::default();
        let package_id = resolve.push_str("test", wit).expect("should parse WIT");
        let world_id = resolve
            .select_world(package_id, Some(world))
            .expect("should select world");

        let mut wasm = wit_component::dummy_module(&resolve, world_id);
        wit_component::embed_component_metadata(
            &mut wasm,
            &resolve,
            world_id,
            wit_component::StringEncoding::UTF8,
        )
        .expect("should embed component metadata");

        let encoder = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&wasm)
            .expect("should set module");
        encoder.encode().expect("should encode component")
    }

    fn annotatable_app() -> LockedApp {
        let mut meta_builder = spin_locked_app::values::ValuesMapBuilder::new();
        meta_builder
            .string("name", "this-is-spinal-tap")
            .string("version", "11.11.11")
            .string("description", "")
            .string_array("authors", vec!["Marty DiBergi", "Artie Fufkin"]);
        let metadata = meta_builder.build();
        LockedApp {
            spin_lock_version: Default::default(),
            must_understand: vec![],
            metadata,
            host_requirements: Default::default(),
            variables: Default::default(),
            triggers: Default::default(),
            components: Default::default(),
        }
    }

    fn as_annotations(annotations: &[(&str, &str)]) -> Option<BTreeMap<String, String>> {
        Some(
            annotations
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn no_annotations_no_infer_result_is_no_annotations() {
        let locked_app = annotatable_app();
        let explicit = None;
        let infer = InferPredefinedAnnotations::None;

        assert!(all_annotations(&locked_app, explicit, infer).is_none());
    }

    #[test]
    fn explicit_annotations_no_infer_result_is_explicit_annotations() {
        let locked_app = annotatable_app();
        let explicit = as_annotations(&[("volume", "11"), ("dimensions", "feet")]);
        let infer = InferPredefinedAnnotations::None;

        let annotations =
            all_annotations(&locked_app, explicit, infer).expect("should still have annotations");
        assert_eq!(2, annotations.len());
        assert_eq!("11", annotations.get("volume").unwrap());
        assert_eq!("feet", annotations.get("dimensions").unwrap());
    }

    #[test]
    fn no_annotations_infer_all_result_is_auto_annotations() {
        let locked_app = annotatable_app();
        let explicit = None;
        let infer = InferPredefinedAnnotations::All;

        let annotations =
            all_annotations(&locked_app, explicit, infer).expect("should now have annotations");
        assert_eq!(4, annotations.len());
        assert_eq!(
            "Marty DiBergi, Artie Fufkin",
            annotations
                .get(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_AUTHORS)
                .expect("should have authors annotation")
        );
        assert_eq!(
            "this-is-spinal-tap",
            annotations
                .get(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_TITLE)
                .expect("should have title annotation")
        );
        assert_eq!(
            "11.11.11",
            annotations
                .get(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_VERSION)
                .expect("should have version annotation")
        );
        assert!(
            !annotations
                .contains_key(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_DESCRIPTION),
            "empty description should not have generated annotation"
        );
        assert!(
            annotations
                .contains_key(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_CREATED),
            "creation annotation should have been generated"
        );
    }

    #[test]
    fn explicit_annotations_infer_all_gets_both_sets() {
        let locked_app = annotatable_app();
        let explicit = as_annotations(&[("volume", "11"), ("dimensions", "feet")]);
        let infer = InferPredefinedAnnotations::All;

        let annotations =
            all_annotations(&locked_app, explicit, infer).expect("should still have annotations");
        assert_eq!(6, annotations.len());
        assert_eq!(
            "11",
            annotations
                .get("volume")
                .expect("should have retained explicit annotation")
        );
        assert_eq!(
            "Marty DiBergi, Artie Fufkin",
            annotations
                .get(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_AUTHORS)
                .expect("should have authors annotation")
        );
    }

    #[test]
    fn explicit_annotations_take_precedence_over_inferred() {
        let locked_app = annotatable_app();
        let explicit = as_annotations(&[
            ("volume", "11"),
            (
                oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_AUTHORS,
                "David St Hubbins, Nigel Tufnel",
            ),
        ]);
        let infer = InferPredefinedAnnotations::All;

        let annotations =
            all_annotations(&locked_app, explicit, infer).expect("should still have annotations");
        assert_eq!(
            5,
            annotations.len(),
            "should have one custom, one predefined explicit, and three inferred"
        );
        assert_eq!(
            "11",
            annotations
                .get("volume")
                .expect("should have retained explicit annotation")
        );
        assert_eq!(
            "David St Hubbins, Nigel Tufnel",
            annotations
                .get(oci_distribution::annotations::ORG_OPENCONTAINERS_IMAGE_AUTHORS)
                .expect("should have authors annotation"),
            "explicit authors should have taken precedence"
        );
    }
}
