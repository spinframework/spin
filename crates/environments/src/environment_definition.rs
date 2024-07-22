use std::{collections::HashMap, path::Path};

use anyhow::Context;
use futures::future::try_join_all;
use spin_common::ui::quoted_path;
use spin_manifest::schema::v2::TargetEnvironmentRef;

const DEFAULT_REGISTRY: &str = "fermyon.com";

/// Serialisation format for the lockfile: registry -> { name -> digest }
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TargetEnvironmentLockfile(HashMap<String, HashMap<String, String>>);

impl TargetEnvironmentLockfile {
    fn digest(&self, registry: &str, env_id: &str) -> Option<&str> {
        self.0
            .get(registry)
            .and_then(|m| m.get(env_id))
            .map(|s| s.as_str())
    }

    fn set_digest(&mut self, registry: &str, env_id: &str, digest: &str) {
        match self.0.get_mut(registry) {
            Some(map) => {
                map.insert(env_id.to_string(), digest.to_string());
            }
            None => {
                let map = vec![(env_id.to_string(), digest.to_string())]
                    .into_iter()
                    .collect();
                self.0.insert(registry.to_string(), map);
            }
        }
    }
}

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
        TargetEnvironmentRef::DefaultRegistry(package) => {
            load_environment_from_registry(DEFAULT_REGISTRY, package, cache, lockfile).await
        }
        TargetEnvironmentRef::Registry { registry, package } => {
            load_environment_from_registry(registry, package, cache, lockfile).await
        }
        TargetEnvironmentRef::WitDirectory { path } => load_environment_from_dir(path),
    }
}

/// Loads the given `TargetEnvironment` from the given registry, or
/// from cache if available. If the environment is not in cache, the
/// encoded WIT will be cached, and the in-memory lockfile object
/// updated.
async fn load_environment_from_registry(
    registry: &str,
    env_id: &str,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment> {
    use futures_util::TryStreamExt;

    if let Some(digest) = lockfile.read().await.digest(registry, env_id) {
        if let Ok(cache_file) = cache.wasm_file(digest) {
            if let Ok(bytes) = tokio::fs::read(&cache_file).await {
                return TargetEnvironment::from_package_bytes(env_id, bytes);
            }
        }
    }

    let (pkg_name, pkg_ver) = env_id.split_once('@').with_context(|| format!("Failed to parse target environment {env_id} as package reference - is the target correct?"))?;
    let env_pkg_ref: wasm_pkg_client::PackageRef = pkg_name
        .parse()
        .with_context(|| format!("Environment {pkg_name} is not a valid package name"))?;

    let wkg_registry: wasm_pkg_client::Registry = registry
        .parse()
        .with_context(|| format!("Registry {registry} is not a valid registry name"))?;

    // TODO: this requires wkg configuration which shouldn't be on users:
    // is there a better way to handle it?
    let mut wkg_config = wasm_pkg_client::Config::global_defaults().await?;
    wkg_config.set_package_registry_override(
        env_pkg_ref,
        wasm_pkg_client::RegistryMapping::Registry(wkg_registry),
    );

    let client = wasm_pkg_client::Client::new(wkg_config);

    let package = pkg_name
        .to_owned()
        .try_into()
        .with_context(|| format!("Failed to parse environment name {pkg_name} as package name"))?;
    let version = wasm_pkg_client::Version::parse(pkg_ver).with_context(|| {
        format!("Failed to parse environment version {pkg_ver} as package version")
    })?;

    let release = client
        .get_release(&package, &version)
        .await
        .with_context(|| format!("Failed to get {env_id} release from registry"))?;
    let stm = client
        .stream_content(&package, &release)
        .await
        .with_context(|| format!("Failed to get {env_id} package from registry"))?;
    let bytes = stm
        .try_collect::<bytes::BytesMut>()
        .await
        .with_context(|| format!("Failed to get {env_id} package data from registry"))?
        .to_vec();

    let digest = release.content_digest.to_string();
    _ = cache.write_wasm(&bytes, &digest).await; // Failure to cache is not fatal
    lockfile.write().await.set_digest(registry, env_id, &digest);

    TargetEnvironment::from_package_bytes(env_id, bytes)
}

fn load_environment_from_dir(path: &Path) -> anyhow::Result<TargetEnvironment> {
    let mut resolve = wit_parser::Resolve::default();
    let (pkg_id, _) = resolve.push_dir(path)?;
    let decoded = wit_parser::decoding::DecodedWasm::WitPackage(resolve, pkg_id);
    TargetEnvironment::from_decoded_wasm(path, decoded)
}

/// A parsed document representing a deployment environment, e.g. Spin 2.7,
/// SpinKube 3.1, Fermyon Cloud. The `TargetEnvironment` provides a mapping
/// from the Spin trigger types supported in the environment to the Component Model worlds
/// supported by that trigger type. (A trigger type may support more than one world,
/// for example when it supports multiple versions of the Spin or WASI interfaces.)
///
/// In terms of implementation, internally the environment is represented by a
/// WIT package that adheres to a specific naming convention (that the worlds for
/// a given trigger type are exactly whose names begin `trigger-xxx` where
/// `xxx` is the Spin trigger type).
pub struct TargetEnvironment {
    name: String,
    decoded: wit_parser::decoding::DecodedWasm,
    package: wit_parser::Package,
    package_id: id_arena::Id<wit_parser::Package>,
    package_bytes: Vec<u8>,
}

impl TargetEnvironment {
    fn from_package_bytes(name: &str, bytes: Vec<u8>) -> anyhow::Result<Self> {
        let decoded = wit_component::decode(&bytes)
            .with_context(|| format!("Failed to decode package for environment {name}"))?;
        let package_id = decoded.package();
        let package = decoded
            .resolve()
            .packages
            .get(package_id)
            .with_context(|| {
                format!("The {name} environment is invalid (no package for decoded package ID)")
            })?
            .clone();

        Ok(Self {
            name: name.to_owned(),
            decoded,
            package,
            package_id,
            package_bytes: bytes,
        })
    }

    fn from_decoded_wasm(
        source: &Path,
        decoded: wit_parser::decoding::DecodedWasm,
    ) -> anyhow::Result<Self> {
        let package_id = decoded.package();
        let package = decoded
            .resolve()
            .packages
            .get(package_id)
            .with_context(|| {
                format!(
                    "The {} environment is invalid (no package for decoded package ID)",
                    quoted_path(source)
                )
            })?
            .clone();
        let name = package.name.to_string();

        let bytes = wit_component::encode(decoded.resolve(), package_id)?;

        Ok(Self {
            name,
            decoded,
            package,
            package_id,
            package_bytes: bytes,
        })
    }

    /// Returns true if the given trigger type provides the world identified by
    /// `world` in this environment.
    pub fn is_world_for(&self, trigger_type: &TriggerType, world: &wit_parser::World) -> bool {
        world.name.starts_with(&format!("trigger-{trigger_type}"))
            && world.package.is_some_and(|p| p == self.package_id)
    }

    /// Returns true if the given trigger type can run in this environment.
    pub fn supports_trigger_type(&self, trigger_type: &TriggerType) -> bool {
        self.decoded
            .resolve()
            .worlds
            .iter()
            .any(|(_, world)| self.is_world_for(trigger_type, world))
    }

    /// Lists all worlds supported for the given trigger type in this environment.
    pub fn worlds(&self, trigger_type: &TriggerType) -> Vec<String> {
        self.decoded
            .resolve()
            .worlds
            .iter()
            .filter(|(_, world)| self.is_world_for(trigger_type, world))
            .map(|(_, world)| self.world_qname(world))
            .collect()
    }

    /// Fully qualified world name (e.g. spin:up/trigger-http@3.2.0)
    fn world_qname(&self, world: &wit_parser::World) -> String {
        let version_suffix = self
            .package_version()
            .map(|version| format!("@{version}"))
            .unwrap_or_default();
        format!(
            "{}/{}{version_suffix}",
            self.package_namespaced_name(),
            world.name,
        )
    }

    /// The environment name for UI purposes
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Namespaced but unversioned package name (e.g. spin:up)
    pub fn package_namespaced_name(&self) -> String {
        format!("{}:{}", self.package.name.namespace, self.package.name.name)
    }

    /// The package version for the environment package.
    pub fn package_version(&self) -> Option<&semver::Version> {
        self.package.name.version.as_ref()
    }

    /// The Wasm-encoded bytes of the environment package.
    pub fn package_bytes(&self) -> &[u8] {
        &self.package_bytes
    }
}

pub type TriggerType = String;
