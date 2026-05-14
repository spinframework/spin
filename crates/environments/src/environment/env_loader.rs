//! Loading target environments, from a list of references through to
//! a fully realised collection of WIT packages with their worlds and
//! mappings.

use std::path::PathBuf;
use std::sync::Arc;
use std::{collections::HashMap, path::Path};

use anyhow::{Context, anyhow};
use futures::future::try_join_all;
use spin_common::ui::quoted_path;
use spin_manifest::schema::v2::TargetEnvironmentRef;

use crate::environment::catalogue::Catalogue;

use super::definition::{EnvironmentDefinition, WorldName, WorldRef};
use super::lockfile::TargetEnvironmentLockfile;
use super::{CandidateWorld, CandidateWorlds, TargetEnvironment, UnknownTrigger};

const DEFAULT_PACKAGE_REGISTRY: &str = "spinframework.dev";

pub struct LoadedEnvironmentDefinition {
    pub name: String,
    pub env_def: EnvironmentDefinition,
    pub relative_path_base: Option<PathBuf>,
}

impl LoadedEnvironmentDefinition {
    fn new(
        name: impl Into<String>,
        env_def: EnvironmentDefinition,
        relative_path_base: Option<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            env_def,
            relative_path_base,
        }
    }
}

/// Load all the listed environments from their registries or paths.
/// Registry data will be cached, with a lockfile under `.spin` mapping
/// environment IDs to digests (to allow cache lookup without needing
/// to fetch the digest from the registry).
pub async fn load_environments<'a>(
    env_ids: &[&'a TargetEnvironmentRef],
    cache_root: Option<std::path::PathBuf>,
    app_dir: &std::path::Path,
) -> anyhow::Result<HashMap<&'a TargetEnvironmentRef, Arc<TargetEnvironment>>> {
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
            .map(|e| load_environment(e, app_dir, &cache, &lockfile)),
    )
    .await?
    .into_iter()
    .map(|(k, v)| (k, Arc::new(v)))
    .collect();

    let final_lockfile = &*lockfile.read().await;
    if *final_lockfile != orig_lockfile
        && let Ok(lockfile_json) = serde_json::to_string_pretty(&final_lockfile)
    {
        _ = tokio::fs::create_dir_all(lockfile_dir).await;
        _ = tokio::fs::write(&lockfile_path, lockfile_json).await; // failure to update lockfile is not an error
    }

    Ok(envs)
}

/// Loads the given `TargetEnvironment` from a registry or directory.
async fn load_environment<'a>(
    env_id: &'a TargetEnvironmentRef,
    app_dir: &Path,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<(&'a TargetEnvironmentRef, TargetEnvironment)> {
    let loaded_env_def = load_environment_def(env_id, app_dir).await?;
    let env = load_environment_from_env_def(loaded_env_def, cache, lockfile).await?;
    Ok((env_id, env))
}

pub async fn load_environment_def(
    env_id: &TargetEnvironmentRef,
    app_dir: &Path,
) -> Result<LoadedEnvironmentDefinition, anyhow::Error> {
    match env_id {
        TargetEnvironmentRef::Catalogue(id) => load_environment_def_from_catalogue(id).await,
        TargetEnvironmentRef::Http { url } => load_environment_def_from_http(url).await,
        TargetEnvironmentRef::File { path } => {
            load_environment_def_from_file(app_dir.join(path)).await
        }
    }
}

/// Loads a `EnvironmentDefinition` from the catalogue. If not found, the catalogue is refreshed
/// and retried. Any remote packages the environment references will be used
/// from cache if available; otherwise, they will be saved to the cache, and the
/// in-memory lockfile object updated.
async fn load_environment_def_from_catalogue(
    env_id: &str,
) -> anyhow::Result<LoadedEnvironmentDefinition> {
    let catalogue = Catalogue::try_default()?;
    let env_id = env_id.replace(':', "@");
    let env_def = match catalogue.get(&env_id).await? {
        Some(env_def) => env_def,
        None => {
            catalogue.update().await?;
            catalogue
                .get(&env_id)
                .await?
                .with_context(|| anyhow!("Cannot load target environment '{env_id}'"))?
        }
    };
    Ok(LoadedEnvironmentDefinition::new(env_id, env_def, None))
}

/// Loads a `EnvironmentDefinition` from the given
/// URL. Any remote packages the environment references will be used
/// from cache if available; otherwise, they will be saved to the cache, and the
/// in-memory lockfile object updated.
async fn load_environment_def_from_http(url: &str) -> anyhow::Result<LoadedEnvironmentDefinition> {
    let toml_text = reqwest::get(url).await?.text().await?;
    let env_def: EnvironmentDefinition = toml::from_str(&toml_text)?;
    let url = url::Url::parse(url)?;
    let env_id = url
        .path_segments()
        .with_context(|| format!("environment URL {url} does not have a path"))?
        .next_back()
        .with_context(|| format!("environment URL {url} does not have a path"))?;
    let env_id = env_id
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(env_id);
    Ok(LoadedEnvironmentDefinition::new(env_id, env_def, None))
}

/// Loads a `EnvironmentDefinition` from the given TOML file. Any remote packages
/// it references will be used from cache if available; otherwise, they will be saved
/// to the cache, and the in-memory lockfile object updated.
async fn load_environment_def_from_file(
    path: impl AsRef<Path>,
) -> anyhow::Result<LoadedEnvironmentDefinition> {
    let path = path.as_ref();
    let env_def_dir = path.parent().map(|p| p.to_owned());
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
    let env_def: EnvironmentDefinition = toml::from_str(&toml_text)?;
    Ok(LoadedEnvironmentDefinition::new(name, env_def, env_def_dir))
}

/// Loads a `TargetEnvironment` from the given TOML text. Any remote packages
/// it references will be used from cache if available; otherwise, they will be saved
/// to the cache, and the in-memory lockfile object updated.
async fn load_environment_from_env_def(
    loaded_env_def: LoadedEnvironmentDefinition,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment> {
    let mut trigger_worlds = HashMap::new();
    let mut trigger_capabilities = HashMap::new();

    let LoadedEnvironmentDefinition {
        name,
        env_def,
        relative_path_base,
    } = loaded_env_def;

    // TODO: parallel all the things
    // TODO: this loads _all_ triggers not just the ones we need
    for (trigger_type, trigger_env) in env_def.triggers() {
        trigger_worlds.insert(
            trigger_type.to_owned(),
            load_worlds(
                trigger_env.world_refs(),
                &relative_path_base,
                cache,
                lockfile,
            )
            .await?,
        );
        trigger_capabilities.insert(trigger_type.to_owned(), trigger_env.capabilities());
    }

    let unknown_trigger = match env_def.default() {
        None => UnknownTrigger::Deny,
        Some(env) => UnknownTrigger::Allow(
            load_worlds(env.world_refs(), &relative_path_base, cache, lockfile).await?,
        ),
    };
    let unknown_capabilities = match env_def.default() {
        None => vec![],
        Some(env) => env.capabilities(),
    };

    Ok(TargetEnvironment {
        name: name.to_owned(),
        trigger_worlds,
        trigger_capabilities,
        unknown_trigger,
        unknown_capabilities,
    })
}

async fn load_worlds(
    world_refs: &[WorldRef],
    relative_to_dir: &Option<PathBuf>,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<CandidateWorlds> {
    let mut worlds = vec![];

    for world_ref in world_refs {
        worlds.push(load_world(world_ref, relative_to_dir, cache, lockfile).await?);
    }

    Ok(CandidateWorlds { worlds })
}

async fn load_world(
    world_ref: &WorldRef,
    relative_to_dir: &Option<PathBuf>,
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
        WorldRef::WitDirectory { path, world } => {
            let path = match relative_to_dir {
                Some(dir) => dir.join(path),
                None => path.to_owned(),
            };
            load_world_from_dir(&path, world)
        }
    }
}

fn load_world_from_dir(
    path: impl AsRef<Path>,
    world: &WorldName,
) -> anyhow::Result<CandidateWorld> {
    let path = path.as_ref();
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
        && let Ok(cache_file) = cache.wasm_file(digest)
        && let Ok(bytes) = tokio::fs::read(&cache_file).await
    {
        return CandidateWorld::from_package_bytes(world_name, bytes);
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
