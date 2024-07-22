//! Loading target environments, from a list of references through to
//! a fully realised collection of WIT packages with their worlds and
//! mappings.

use std::{collections::HashMap, path::Path};

use anyhow::{anyhow, Context};
use futures::future::try_join_all;
use spin_common::ui::quoted_path;
use spin_manifest::schema::v2::TargetEnvironmentRef;

use super::definition::{EnvironmentDefinition, WorldName, WorldRef};
use super::lockfile::TargetEnvironmentLockfile;
use super::{is_versioned, CandidateWorld, CandidateWorlds, TargetEnvironment, UnknownTrigger};

const DEFAULT_ENV_DEF_REGISTRY_PREFIX: &str = "ghcr.io/spinframework/environments";
const DEFAULT_PACKAGE_REGISTRY: &str = "spinframework.dev";

/// Load all the listed environments from their registries or paths.
/// Registry data will be cached, with a lockfile under `.spin` mapping
/// environment IDs to digests (to allow cache lookup without needing
/// to fetch the digest from the registry).
pub async fn load_environments(
    env_ids: &[TargetEnvironmentRef],
    cache_root: Option<std::path::PathBuf>,
    app_dir: &std::path::Path,
) -> anyhow::Result<Vec<TargetEnvironment>> {
    if env_ids.is_empty() {
        return Ok(Default::default());
    }

    let cache = spin_loader::cache::Cache::new(cache_root)
        .await
        .context("Unable to create cache")?;
    let lockfile_dir = app_dir.join(".spin");
    let lockfile_path = lockfile_dir.join("target-environments.lock");

    let orig_lockfile: TargetEnvironmentLockfile = tokio::fs::read_to_string(&lockfile_path)
        .await
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let lockfile = std::sync::Arc::new(tokio::sync::RwLock::new(orig_lockfile.clone()));

    let envs = try_join_all(
        env_ids
            .iter()
            .map(|e| load_environment(e, &cache, &lockfile)),
    )
    .await?;

    let final_lockfile = &*lockfile.read().await;
    if *final_lockfile != orig_lockfile {
        if let Ok(lockfile_json) = serde_json::to_string_pretty(&final_lockfile) {
            _ = tokio::fs::create_dir_all(lockfile_dir).await;
            _ = tokio::fs::write(&lockfile_path, lockfile_json).await; // failure to update lockfile is not an error
        }
    }

    Ok(envs)
}

/// Loads the given `TargetEnvironment` from a registry or directory.
async fn load_environment(
    env_id: &TargetEnvironmentRef,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment> {
    match env_id {
        TargetEnvironmentRef::DefaultRegistry(id) => {
            load_environment_from_registry(DEFAULT_ENV_DEF_REGISTRY_PREFIX, id, cache, lockfile)
                .await
        }
        TargetEnvironmentRef::Registry { registry, id } => {
            load_environment_from_registry(registry, id, cache, lockfile).await
        }
        TargetEnvironmentRef::File { path } => {
            load_environment_from_file(path, cache, lockfile).await
        }
    }
}

/// Loads a `TargetEnvironment` from the environment definition at the given
/// registry location. The environment and any remote packages it references will be used
/// from cache if available; otherwise, they will be saved to the cache, and the
/// in-memory lockfile object updated.
async fn load_environment_from_registry(
    registry: &str,
    env_id: &str,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment> {
    let env_def_toml = load_env_def_toml_from_registry(registry, env_id, cache, lockfile).await?;
    load_environment_from_toml(env_id, &env_def_toml, cache, lockfile).await
}

/// Loads a `TargetEnvironment` from the given TOML file. Any remote packages
/// it references will be used from cache if available; otherwise, they will be saved
/// to the cache, and the in-memory lockfile object updated.
async fn load_environment_from_file(
    path: &Path,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_owned())
        .unwrap();
    let toml_text = tokio::fs::read_to_string(path).await.with_context(|| {
        format!(
            "unable to read target environment from {}",
            quoted_path(path)
        )
    })?;
    load_environment_from_toml(&name, &toml_text, cache, lockfile).await
}

/// Loads a `TargetEnvironment` from the given TOML text. Any remote packages
/// it references will be used from cache if available; otherwise, they will be saved
/// to the cache, and the in-memory lockfile object updated.
async fn load_environment_from_toml(
    name: &str,
    toml_text: &str,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment> {
    let env: EnvironmentDefinition = toml::from_str(toml_text)?;

    let mut trigger_worlds = HashMap::new();

    // TODO: parallel all the things
    // TODO: this loads _all_ triggers not just the ones we need
    for (trigger_type, world_refs) in env.triggers() {
        trigger_worlds.insert(
            trigger_type.to_owned(),
            load_worlds(world_refs, cache, lockfile).await?,
        );
    }

    let unknown_trigger = match env.default() {
        None => UnknownTrigger::Deny,
        Some(world_refs) => UnknownTrigger::Allow(load_worlds(world_refs, cache, lockfile).await?),
    };

    Ok(TargetEnvironment {
        name: name.to_owned(),
        trigger_worlds,
        unknown_trigger,
    })
}

/// Loads the text (assumed to be TOML) from the environment definition at the given
/// registry location. The environment will be used from cache if available; otherwise,
/// it be saved to the cache, and the in-memory lockfile object updated.
async fn load_env_def_toml_from_registry(
    registry: &str,
    env_id: &str,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<String> {
    if let Some(digest) = lockfile.read().await.env_digest(registry, env_id) {
        if let Ok(cache_file) = cache.data_file(digest) {
            if let Ok(bytes) = tokio::fs::read(&cache_file).await {
                return Ok(String::from_utf8_lossy(&bytes).to_string());
            }
        }
    }

    let (bytes, digest) = download_env_def_file(registry, env_id)
        .await
        .with_context(|| format!("downloading target environment {env_id} from {registry}"))?;

    let toml_text = String::from_utf8_lossy(&bytes).to_string();

    _ = cache.write_data(bytes, &digest).await;
    lockfile
        .write()
        .await
        .set_env_digest(registry, env_id, &digest);

    Ok(toml_text)
}

/// Downloads a single-layer document from the given registry.
/// (You can create a suitable document with e.g. `oras push ghcr.io/my/envs/sample:1.0 sample.toml`.)
/// The image must be publicly accessible (which is *NOT* the default with GHCR).
///
/// The return value is a tuple of (content, digest).
async fn download_env_def_file(registry: &str, env_id: &str) -> anyhow::Result<(Vec<u8>, String)> {
    // This implies env_id is in the format spin-up:3.2
    let registry_id = if is_versioned(env_id) {
        env_id.to_string()
    } else {
        // Testing versionless tags with GHCR it didn't work
        // TODO: is this expected or am I being a dolt
        // TODO: is this a suitable workaround
        format!("{env_id}:latest")
    };

    let reference = format!("{registry}/{registry_id}");
    let reference = oci_distribution::Reference::try_from(reference)?;

    let config = oci_distribution::client::ClientConfig::default();
    let client = oci_distribution::client::Client::new(config);
    let auth = oci_distribution::secrets::RegistryAuth::Anonymous;

    let (manifest, digest) = client.pull_manifest(&reference, &auth).await?;

    let im = match manifest {
        oci_distribution::manifest::OciManifest::Image(im) => im,
        oci_distribution::manifest::OciManifest::ImageIndex(_) => {
            anyhow::bail!("unexpected registry format for {reference}")
        }
    };

    let count = im.layers.len();

    if count != 1 {
        anyhow::bail!("artifact {reference} should have had exactly one layer");
    }

    let the_layer = &im.layers[0];
    let mut out = Vec::with_capacity(the_layer.size.try_into().unwrap_or_default());
    client.pull_blob(&reference, the_layer, &mut out).await?;

    Ok((out, digest))
}

async fn load_worlds(
    world_refs: &[WorldRef],
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<CandidateWorlds> {
    let mut worlds = vec![];

    for world_ref in world_refs {
        worlds.push(load_world(world_ref, cache, lockfile).await?);
    }

    Ok(CandidateWorlds { worlds })
}

async fn load_world(
    world_ref: &WorldRef,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<CandidateWorld> {
    match world_ref {
        WorldRef::DefaultRegistry(world) => {
            load_world_from_registry(DEFAULT_PACKAGE_REGISTRY, world, cache, lockfile).await
        }
        WorldRef::Registry { registry, world } => {
            load_world_from_registry(registry, world, cache, lockfile).await
        }
        WorldRef::WitDirectory { path, world } => load_world_from_dir(path, world),
    }
}

fn load_world_from_dir(path: &Path, world: &WorldName) -> anyhow::Result<CandidateWorld> {
    let mut resolve = wit_parser::Resolve::default();
    let (pkg_id, _) = resolve.push_dir(path)?;
    let decoded = wit_parser::decoding::DecodedWasm::WitPackage(resolve, pkg_id);
    CandidateWorld::from_decoded_wasm(world, path, decoded)
}

/// Loads the given `TargetEnvironment` from the given registry, or
/// from cache if available. If the environment is not in cache, the
/// encoded WIT will be cached, and the in-memory lockfile object
/// updated.
async fn load_world_from_registry(
    registry: &str,
    world_name: &WorldName,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<CandidateWorld> {
    use futures_util::TryStreamExt;

    if let Some(digest) = lockfile
        .read()
        .await
        .package_digest(registry, world_name.package())
    {
        if let Ok(cache_file) = cache.wasm_file(digest) {
            if let Ok(bytes) = tokio::fs::read(&cache_file).await {
                return CandidateWorld::from_package_bytes(world_name, bytes);
            }
        }
    }

    let pkg_name = world_name.package_namespaced_name();
    let pkg_ref = world_name.package_ref()?;

    let wkg_registry: wasm_pkg_client::Registry = registry
        .parse()
        .with_context(|| format!("Registry {registry} is not a valid registry name"))?;

    let mut wkg_config = wasm_pkg_client::Config::global_defaults().await?;
    wkg_config.set_package_registry_override(
        pkg_ref,
        wasm_pkg_client::RegistryMapping::Registry(wkg_registry),
    );

    let client = wasm_pkg_client::Client::new(wkg_config);

    let package = pkg_name
        .to_owned()
        .try_into()
        .with_context(|| format!("Failed to parse environment name {pkg_name} as package name"))?;
    let version = world_name
        .package_version() // TODO: surely we can cope with worlds from unversioned packages? surely?
        .ok_or_else(|| anyhow!("{world_name} is unversioned: this is not currently supported"))?;

    let release = client
        .get_release(&package, version)
        .await
        .with_context(|| format!("Failed to get {} from registry", world_name.package()))?;
    let stm = client
        .stream_content(&package, &release)
        .await
        .with_context(|| format!("Failed to get {} from registry", world_name.package()))?;
    let bytes = stm
        .try_collect::<bytes::BytesMut>()
        .await
        .with_context(|| format!("Failed to get {} from registry", world_name.package()))?
        .to_vec();

    let digest = release.content_digest.to_string();
    _ = cache.write_wasm(&bytes, &digest).await; // Failure to cache is not fatal
    lockfile
        .write()
        .await
        .set_package_digest(registry, world_name.package(), &digest);

    CandidateWorld::from_package_bytes(world_name, bytes)
}
