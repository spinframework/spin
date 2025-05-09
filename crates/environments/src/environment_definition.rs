use std::{collections::HashMap, path::Path};

use anyhow::Context;
use futures::future::try_join_all;
use spin_common::ui::quoted_path;
use spin_manifest::schema::v2::TargetEnvironmentRef2;

const DEFAULT_ENV_DEF_REGISTRY: &str = "ghcr.io/itowlson/envs";
const DEFAULT_PACKAGE_REGISTRY: &str = "spinframework.dev";

/// Serialisation format for the lockfile: registry -> env|pkg -> { name -> digest }
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TargetEnvironmentLockfile(HashMap<String, Digests>);

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Digests {
    env: HashMap<String, String>,
    package: HashMap<String, String>,
}

impl TargetEnvironmentLockfile {
    fn env_digest(&self, registry: &str, env_id: &str) -> Option<&str> {
        self.0
            .get(registry)
            .and_then(|ds| ds.env.get(env_id))
            .map(|s| s.as_str())
    }

    fn set_env_digest(&mut self, registry: &str, env_id: &str, digest: &str) {
        match self.0.get_mut(registry) {
            Some(ds) => {
                ds.env.insert(env_id.to_string(), digest.to_string());
            }
            None => {
                let map = vec![(env_id.to_string(), digest.to_string())]
                    .into_iter()
                    .collect();
                let ds = Digests {
                    env: map,
                    package: Default::default(),
                };
                self.0.insert(registry.to_string(), ds);
            }
        }
    }

    fn package_digest(&self, registry: &str, package: &wit_parser::PackageName) -> Option<&str> {
        self.0
            .get(registry)
            .and_then(|ds| ds.package.get(&package.to_string()))
            .map(|s| s.as_str())
    }

    fn set_package_digest(
        &mut self,
        registry: &str,
        package: &wit_parser::PackageName,
        digest: &str,
    ) {
        match self.0.get_mut(registry) {
            Some(ds) => {
                ds.package.insert(package.to_string(), digest.to_string());
            }
            None => {
                let map = vec![(package.to_string(), digest.to_string())]
                    .into_iter()
                    .collect();
                let ds = Digests {
                    env: Default::default(),
                    package: map,
                };
                self.0.insert(registry.to_string(), ds);
            }
        }
    }
}

/// Load all the listed environments from their registries or paths.
/// Registry data will be cached, with a lockfile under `.spin` mapping
/// environment IDs to digests (to allow cache lookup without needing
/// to fetch the digest from the registry).
pub async fn load_environments(
    env_ids: &[TargetEnvironmentRef2],
    cache_root: Option<std::path::PathBuf>,
    app_dir: &std::path::Path,
) -> anyhow::Result<Vec<TargetEnvironment2>> {
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
    env_id: &TargetEnvironmentRef2,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment2> {
    match env_id {
        TargetEnvironmentRef2::DefaultRegistry(id) => {
            load_environment_from_registry(DEFAULT_ENV_DEF_REGISTRY, id, cache, lockfile).await
        }
        TargetEnvironmentRef2::Registry { registry, id } => {
            load_environment_from_registry(registry, id, cache, lockfile).await
        }
        TargetEnvironmentRef2::File { path } => {
            load_environment_from_file(path, cache, lockfile).await
        }
    }
}

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

    let (bytes, digest) = download_env_def_file(registry, env_id).await?;

    let toml_text = String::from_utf8_lossy(&bytes).to_string();

    _ = cache.write_data(bytes, &digest).await;
    lockfile
        .write()
        .await
        .set_env_digest(registry, env_id, &digest);

    Ok(toml_text)
}

async fn download_env_def_file(registry: &str, env_id: &str) -> anyhow::Result<(Vec<u8>, String)> {
    // This implies env_id is in the format spin-up:3.2 which WHO KNOWS
    let reference = format!("{registry}/{env_id}");
    let reference = oci_distribution::Reference::try_from(reference)?;

    let config = oci_distribution::client::ClientConfig::default();
    let client = oci_distribution::client::Client::new(config);
    let auth = oci_distribution::secrets::RegistryAuth::Anonymous;

    let (manifest, digest) = client.pull_manifest(&reference, &auth).await?;

    let im = match manifest {
        oci_distribution::manifest::OciManifest::Image(im) => im,
        oci_distribution::manifest::OciManifest::ImageIndex(_ind) => {
            anyhow::bail!("found image index instead of image manifest, get in the sea")
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

/// Loads the given `TargetEnvironment` from the given registry, or
/// from cache if available. If the environment is not in cache, the
/// encoded WIT will be cached, and the in-memory lockfile object
/// updated.
async fn load_environment_from_registry(
    registry: &str,
    env_id: &str,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment2> {
    // use futures_util::TryStreamExt;

    let env_def_toml = load_env_def_toml_from_registry(registry, env_id, cache, lockfile).await?;

    load_environment_from_toml(env_id, &env_def_toml, cache, lockfile).await

    // let (pkg_name, pkg_ver) = env_id.split_once('@').with_context(|| format!("Failed to parse target environment {env_id} as package reference - is the target correct?"))?;
    // let env_pkg_ref: wasm_pkg_client::PackageRef = pkg_name
    //     .parse()
    //     .with_context(|| format!("Environment {pkg_name} is not a valid package name"))?;

    // let wkg_registry: wasm_pkg_client::Registry = registry
    //     .parse()
    //     .with_context(|| format!("Registry {registry} is not a valid registry name"))?;

    // // TODO: this requires wkg configuration which shouldn't be on users:
    // // is there a better way to handle it?
    // let mut wkg_config = wasm_pkg_client::Config::global_defaults().await?;
    // wkg_config.set_package_registry_override(
    //     env_pkg_ref,
    //     wasm_pkg_client::RegistryMapping::Registry(wkg_registry),
    // );

    // let client = wasm_pkg_client::Client::new(wkg_config);

    // let package = pkg_name
    //     .to_owned()
    //     .try_into()
    //     .with_context(|| format!("Failed to parse environment name {pkg_name} as package name"))?;
    // let version = wasm_pkg_client::Version::parse(pkg_ver).with_context(|| {
    //     format!("Failed to parse environment version {pkg_ver} as package version")
    // })?;

    // let release = client
    //     .get_release(&package, &version)
    //     .await
    //     .with_context(|| format!("Failed to get {env_id} release from registry"))?;
    // let stm = client
    //     .stream_content(&package, &release)
    //     .await
    //     .with_context(|| format!("Failed to get {env_id} package from registry"))?;
    // let bytes = stm
    //     .try_collect::<bytes::BytesMut>()
    //     .await
    //     .with_context(|| format!("Failed to get {env_id} package data from registry"))?
    //     .to_vec();

    // let digest = release.content_digest.to_string();
    // _ = cache.write_wasm(&bytes, &digest).await; // Failure to cache is not fatal
    // lockfile.write().await.set_digest(registry, env_id, &digest);

    // TargetEnvironment::from_package_bytes(env_id, bytes)
}

async fn load_worlds(
    world_refs: &[WorldRef],
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<CompatibleWorlds> {
    let mut worlds = vec![];

    for world_ref in world_refs {
        worlds.push(load_world(world_ref, cache, lockfile).await?);
    }

    Ok(CompatibleWorlds { worlds })
}

async fn load_world(
    world_ref: &WorldRef,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<CompatibleWorld> {
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

async fn load_environment_from_file(
    path: &Path,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment2> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_owned())
        .unwrap();
    let toml_text = tokio::fs::read_to_string(path).await?;
    load_environment_from_toml(&name, &toml_text, cache, lockfile).await
}

async fn load_environment_from_toml(
    name: &str,
    toml_text: &str,
    cache: &spin_loader::cache::Cache,
    lockfile: &std::sync::Arc<tokio::sync::RwLock<TargetEnvironmentLockfile>>,
) -> anyhow::Result<TargetEnvironment2> {
    let env: EnvironmentDefinition = toml::from_str(toml_text)?;

    let mut trigger_worlds = HashMap::new();

    // TODO: parallel all the things
    // TODO: this loads _all_ triggers not just the ones we need
    for (trigger_type, world_refs) in env.triggers {
        trigger_worlds.insert(
            trigger_type,
            load_worlds(&world_refs, cache, lockfile).await?,
        );
    }

    let unknown_trigger = match env.default {
        None => UnknownTrigger::Deny,
        Some(world_refs) => UnknownTrigger::Allow(load_worlds(&world_refs, cache, lockfile).await?),
    };

    Ok(TargetEnvironment2 {
        name: name.to_owned(),
        trigger_worlds,
        unknown_trigger,
    })
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(try_from = "String")]
struct WorldName {
    package: wit_parser::PackageName,
    world: String,
}

impl TryFrom<String> for WorldName {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        use wasmparser::names::{ComponentName, ComponentNameKind};

        // World qnames have the same syntactic form as interface qnames
        let parsed = ComponentName::new(&value, 0)?;
        let ComponentNameKind::Interface(itf) = parsed.kind() else {
            anyhow::bail!("{value} is not a well-formed world name");
        };

        let package = wit_parser::PackageName {
            namespace: itf.namespace().to_string(),
            name: itf.package().to_string(),
            version: itf.version(),
        };

        let world = itf.interface().to_string();

        Ok(Self { package, world })
    }
}

impl std::fmt::Display for WorldName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.package.namespace)?;
        f.write_str(":")?;
        f.write_str(&self.package.name)?;
        f.write_str("/")?;
        f.write_str(&self.world)?;

        if let Some(v) = self.package.version.as_ref() {
            f.write_str("@")?;
            f.write_str(&v.to_string())?;
        }

        Ok(())
    }
}

fn load_world_from_dir(path: &Path, world: &WorldName) -> anyhow::Result<CompatibleWorld> {
    let mut resolve = wit_parser::Resolve::default();
    let (pkg_id, _) = resolve.push_dir(path)?;
    let decoded = wit_parser::decoding::DecodedWasm::WitPackage(resolve, pkg_id);
    CompatibleWorld::from_decoded_wasm(world, path, decoded)
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
) -> anyhow::Result<CompatibleWorld> {
    use futures_util::TryStreamExt;

    // TODO: the lookup should presumably be a package name
    if let Some(digest) = lockfile
        .read()
        .await
        .package_digest(registry, &world_name.package)
    {
        if let Ok(cache_file) = cache.wasm_file(digest) {
            if let Ok(bytes) = tokio::fs::read(&cache_file).await {
                return CompatibleWorld::from_package_bytes(world_name, bytes);
            }
        }
    }

    let pkg_name = format!(
        "{}:{}",
        world_name.package.namespace, world_name.package.name
    );
    let pkg_ref: wasm_pkg_client::PackageRef = pkg_name
        .parse()
        .with_context(|| format!("Environment {pkg_name} is not a valid package name"))?;

    let wkg_registry: wasm_pkg_client::Registry = registry
        .parse()
        .with_context(|| format!("Registry {registry} is not a valid registry name"))?;

    // TODO: this requires wkg configuration which shouldn't be on users:
    // is there a better way to handle it?
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
    let version = world_name.package.version.as_ref().unwrap(); // TODO: surely we can cope with unversioned? surely?

    let release = client
        .get_release(&package, version)
        .await
        .with_context(|| format!("Failed to get {} release from registry", world_name.package))?;
    let stm = client
        .stream_content(&package, &release)
        .await
        .with_context(|| format!("Failed to get {} release from registry", world_name.package))?;
    let bytes = stm
        .try_collect::<bytes::BytesMut>()
        .await
        .with_context(|| format!("Failed to get {} release from registry", world_name.package))?
        .to_vec();

    let digest = release.content_digest.to_string();
    _ = cache.write_wasm(&bytes, &digest).await; // Failure to cache is not fatal
    lockfile
        .write()
        .await
        .set_package_digest(registry, &world_name.package, &digest);

    CompatibleWorld::from_package_bytes(world_name, bytes)
}

/// A fully realised deployment environment, e.g. Spin 2.7,
/// SpinKube 3.1, Fermyon Cloud. The `TargetEnvironment` provides a mapping
/// from the Spin trigger types supported in the environment to the Component Model worlds
/// supported by that trigger type. (A trigger type may support more than one world,
/// for example when it supports multiple versions of the Spin or WASI interfaces.)
/// The structure stores all worlds (that is, the packages containing them) as binaries:
/// no further download or resolution is required after this point.
pub struct TargetEnvironment2 {
    name: String,
    trigger_worlds: HashMap<String, CompatibleWorlds>,
    unknown_trigger: UnknownTrigger,
}

impl TargetEnvironment2 {
    /// The environment name for UI purposes
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns true if the given trigger type can run in this environment.
    pub fn supports_trigger_type(&self, trigger_type: &TriggerType) -> bool {
        self.unknown_trigger.allows(trigger_type) || self.trigger_worlds.contains_key(trigger_type)
    }

    /// Lists all worlds supported for the given trigger type in this environment.
    pub fn worlds(&self, trigger_type: &TriggerType) -> &CompatibleWorlds {
        self.trigger_worlds
            .get(trigger_type)
            .or_else(|| self.unknown_trigger.worlds())
            .unwrap_or(NO_COMPATIBLE_WORLDS)
    }
}

enum UnknownTrigger {
    Deny,
    Allow(CompatibleWorlds),
}

impl UnknownTrigger {
    fn allows(&self, _trigger_type: &TriggerType) -> bool {
        matches!(self, Self::Allow(_))
    }

    fn worlds(&self) -> Option<&CompatibleWorlds> {
        match self {
            Self::Deny => None,
            Self::Allow(cw) => Some(cw),
        }
    }
}

#[derive(Default)]
pub struct CompatibleWorlds {
    worlds: Vec<CompatibleWorld>,
}

impl<'a> IntoIterator for &'a CompatibleWorlds {
    type Item = &'a CompatibleWorld;

    type IntoIter = std::slice::Iter<'a, CompatibleWorld>;

    fn into_iter(self) -> Self::IntoIter {
        self.worlds.iter()
    }
}

const NO_COMPATIBLE_WORLDS: &CompatibleWorlds = &CompatibleWorlds { worlds: vec![] };

pub struct CompatibleWorld {
    world: WorldName,
    package: wit_parser::Package,
    // package_id: id_arena::Id<wit_parser::Package>,
    package_bytes: Vec<u8>,
}

impl std::fmt::Display for CompatibleWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.world.fmt(f)
    }
}

impl CompatibleWorld {
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

    fn from_package_bytes(world: &WorldName, bytes: Vec<u8>) -> anyhow::Result<Self> {
        let decoded = wit_component::decode(&bytes)
            .with_context(|| format!("Failed to decode package for environment {world}"))?;
        let package_id = decoded.package();
        let package = decoded
            .resolve()
            .packages
            .get(package_id)
            .with_context(|| {
                format!("The {world} package is invalid (no package for decoded package ID)")
            })?
            .clone();

        Ok(Self {
            world: world.to_owned(),
            package,
            package_bytes: bytes,
        })
    }

    fn from_decoded_wasm(
        world: &WorldName,
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

        let bytes = wit_component::encode(decoded.resolve(), package_id)?;

        Ok(Self {
            world: world.to_owned(),
            package,
            package_bytes: bytes,
        })
    }
}

// The document format

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentDefinition {
    triggers: HashMap<String, Vec<WorldRef>>,
    default: Option<Vec<WorldRef>>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum WorldRef {
    DefaultRegistry(WorldName),
    Registry {
        registry: String,
        world: WorldName,
    },
    WitDirectory {
        path: std::path::PathBuf,
        world: WorldName,
    },
}

// impl TargetEnvironment {
//     fn from_package_bytes(name: &str, bytes: Vec<u8>) -> anyhow::Result<Self> {
//         let decoded = wit_component::decode(&bytes)
//             .with_context(|| format!("Failed to decode package for environment {name}"))?;
//         let package_id = decoded.package();
//         let package = decoded
//             .resolve()
//             .packages
//             .get(package_id)
//             .with_context(|| {
//                 format!("The {name} environment is invalid (no package for decoded package ID)")
//             })?
//             .clone();

//         Ok(Self {
//             name: name.to_owned(),
//             decoded,
//             package,
//             package_id,
//             package_bytes: bytes,
//         })
//     }

//     fn from_decoded_wasm(
//         source: &Path,
//         decoded: wit_parser::decoding::DecodedWasm,
//     ) -> anyhow::Result<Self> {
//         let package_id = decoded.package();
//         let package = decoded
//             .resolve()
//             .packages
//             .get(package_id)
//             .with_context(|| {
//                 format!(
//                     "The {} environment is invalid (no package for decoded package ID)",
//                     quoted_path(source)
//                 )
//             })?
//             .clone();
//         let name = package.name.to_string();

//         let bytes = wit_component::encode(decoded.resolve(), package_id)?;

//         Ok(Self {
//             name,
//             decoded,
//             package,
//             package_id,
//             package_bytes: bytes,
//         })
//     }

//     /// Returns true if the given trigger type provides the world identified by
//     /// `world` in this environment.
//     pub fn is_world_for(&self, trigger_type: &TriggerType, world: &wit_parser::World) -> bool {
//         self.matches_world_name(trigger_type, world)
//             && world.package.is_some_and(|p| p == self.package_id)
//     }

//     fn matches_world_name(&self, trigger_type: &TriggerType, world: &wit_parser::World) -> bool {
//         world.name.starts_with(&format!("trigger-{trigger_type}"))
//             || world.name.starts_with(&format!("{trigger_type}-trigger"))
//             || world.name.starts_with(&format!("spin-{trigger_type}"))
//     }

//     /// Returns true if the given trigger type can run in this environment.
//     pub fn supports_trigger_type(&self, trigger_type: &TriggerType) -> bool {
//         self.decoded
//             .resolve()
//             .worlds
//             .iter()
//             .any(|(_, world)| self.is_world_for(trigger_type, world))
//     }

//     /// Lists all worlds supported for the given trigger type in this environment.
//     pub fn worlds(&self, trigger_type: &TriggerType) -> Vec<String> {
//         self.decoded
//             .resolve()
//             .worlds
//             .iter()
//             .filter(|(_, world)| self.is_world_for(trigger_type, world))
//             .map(|(_, world)| self.world_qname(world))
//             .collect()
//     }

//     /// Fully qualified world name (e.g. spin:up/trigger-http@3.2.0)
//     fn world_qname(&self, world: &wit_parser::World) -> String {
//         let version_suffix = self
//             .package_version()
//             .map(|version| format!("@{version}"))
//             .unwrap_or_default();
//         format!(
//             "{}/{}{version_suffix}",
//             self.package_namespaced_name(),
//             world.name,
//         )
//     }

//     /// The environment name for UI purposes
//     pub fn name(&self) -> &str {
//         &self.name
//     }

//     /// Namespaced but unversioned package name (e.g. spin:up)
//     pub fn package_namespaced_name(&self) -> String {
//         format!("{}:{}", self.package.name.namespace, self.package.name.name)
//     }

//     /// The package version for the environment package.
//     pub fn package_version(&self) -> Option<&semver::Version> {
//         self.package.name.version.as_ref()
//     }

//     /// The Wasm-encoded bytes of the environment package.
//     pub fn package_bytes(&self) -> &[u8] {
//         &self.package_bytes
//     }
// }

pub type TriggerType = String;
