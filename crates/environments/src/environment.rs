use std::{collections::HashMap, path::Path};

use anyhow::Context;
use spin_common::ui::quoted_path;
use spin_manifest::schema::v2::TargetEnvironmentRef;

mod definition;
mod env_loader;
mod lockfile;

use definition::WorldName;

/// A fully realised deployment environment, e.g. Spin 2.7,
/// SpinKube 3.1, Fermyon Cloud. The `TargetEnvironment` provides a mapping
/// from the Spin trigger types supported in the environment to the Component Model worlds
/// supported by that trigger type. (A trigger type may support more than one world,
/// for example when it supports multiple versions of the Spin or WASI interfaces.)
pub struct TargetEnvironment {
    name: String,
    trigger_worlds: HashMap<TriggerType, CandidateWorlds>,
    trigger_capabilities: HashMap<TriggerType, Vec<String>>,
    unknown_trigger: UnknownTrigger,
    unknown_capabilities: Vec<String>,
}

impl TargetEnvironment {
    /// Loads the specified list of environments. This fetches all required
    /// environment definitions from their references, and then chases packages
    /// references until the entire target environment is fully loaded.
    /// The function also caches registry references in the application directory,
    /// to avoid loading from the network when the app is validated again.
    pub async fn load_all(
        env_ids: &[TargetEnvironmentRef],
        cache_root: Option<std::path::PathBuf>,
        app_dir: &std::path::Path,
    ) -> anyhow::Result<Vec<Self>> {
        env_loader::load_environments(env_ids, cache_root, app_dir).await
    }

    /// The environment name for UI purposes
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns true if the given trigger type can run in this environment.
    pub fn supports_trigger_type(&self, trigger_type: &TriggerType) -> bool {
        self.unknown_trigger.allows(trigger_type) || self.trigger_worlds.contains_key(trigger_type)
    }

    /// Lists all worlds supported for the given trigger type in this environment.
    pub fn worlds(&self, trigger_type: &TriggerType) -> &CandidateWorlds {
        self.trigger_worlds
            .get(trigger_type)
            .or_else(|| self.unknown_trigger.worlds())
            .unwrap_or(NO_WORLDS)
    }

    /// Lists all host capabilities supported for the given trigger type in this environment.
    pub fn capabilities(&self, trigger_type: &TriggerType) -> &[String] {
        self.trigger_capabilities
            .get(trigger_type)
            .unwrap_or(&self.unknown_capabilities)
    }
}

/// How a `TargetEnvironment` should validate components associated with trigger types
/// not listed in the/ environment definition. This is used for best-effort validation in
/// extensible environments.
///
/// For example, a "forgiving" definition of Spin CLI environment would
/// validate that components associated with `cron` or `sqs` triggers adhere
/// to the platform world, even though it cannot validate that the exports are correct
/// or that the plugins are installed or up to date. This can result in failure at
/// runtime, but that may be better than refusing to let cron jobs run!
///
/// On the other hand, the SpinKube environment rejects unknown triggers
/// because SpinKube does not allow arbitrary triggers to be linked at
/// runtime: the set of triggers is static for a given version.
enum UnknownTrigger {
    /// Components for unknown trigger types fail validation.
    Deny,
    /// Components for unknown trigger types pass validation if they
    /// conform to (at least) one of the listed worlds.
    Allow(CandidateWorlds),
}

impl UnknownTrigger {
    fn allows(&self, _trigger_type: &TriggerType) -> bool {
        matches!(self, Self::Allow(_))
    }

    fn worlds(&self) -> Option<&CandidateWorlds> {
        match self {
            Self::Deny => None,
            Self::Allow(cw) => Some(cw),
        }
    }
}

/// The set of worlds that a particular trigger type (in a given environment)
/// can accept. For example, the Spin 3.2 CLI `http` trigger accepts various
/// versions of the `spin:up/http-trigger` world.
///
/// A component will pass target validation if it conforms to
/// at least one of these worlds.
#[derive(Default)]
pub struct CandidateWorlds {
    worlds: Vec<CandidateWorld>,
}

impl<'a> IntoIterator for &'a CandidateWorlds {
    type Item = &'a CandidateWorld;

    type IntoIter = std::slice::Iter<'a, CandidateWorld>;

    fn into_iter(self) -> Self::IntoIter {
        self.worlds.iter()
    }
}

const NO_WORLDS: &CandidateWorlds = &CandidateWorlds { worlds: vec![] };

/// A WIT world; specifically, a WIT world provided by a Spin host, against which
/// a component can be validated.
pub struct CandidateWorld {
    world: WorldName,
    package: wit_parser::Package,
    package_bytes: Vec<u8>,
}

impl std::fmt::Display for CandidateWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.world.fmt(f)
    }
}

impl CandidateWorld {
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

pub(super) fn is_versioned(env_id: &str) -> bool {
    env_id.contains(':')
}

pub type TriggerType = String;

#[cfg(test)]
mod test {
    use super::*;

    use std::path::PathBuf;

    const SIMPLE_WIT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/simple-wit");

    /// Construct a CandidateWorlds that matches only the named" world.
    fn load_simple_world(wit_path: &Path, world: &str) -> CandidateWorlds {
        let mut resolve = wit_parser::Resolve::default();
        let (id, _) = resolve
            .push_dir(wit_path)
            .expect("should have pushed WIT dir");
        let package_bytes =
            wit_component::encode(&resolve, id).expect("should have encoded world package");

        let world_name = WorldName::try_from(world.to_owned()).unwrap();
        let simple_world = CandidateWorld::from_package_bytes(&world_name, package_bytes)
            .expect("should have loaded world package");

        CandidateWorlds {
            worlds: vec![simple_world],
        }
    }

    /// Build an environment using the given WIT that maps the "s" trigger
    /// to the "spin:test/simple@1.0.0" world (and denies all other triggers).
    fn target_simple_world(wit_path: &Path) -> TargetEnvironment {
        let candidate_worlds = load_simple_world(wit_path, "spin:test/simple@1.0.0");

        TargetEnvironment {
            name: "test".to_owned(),
            trigger_worlds: [("s".to_owned(), candidate_worlds)].into_iter().collect(),
            trigger_capabilities: Default::default(),
            unknown_trigger: UnknownTrigger::Deny,
            unknown_capabilities: Default::default(),
        }
    }

    /// Build an environment using the given WIT that maps all triggers to
    /// the "spin:test/simple-import-only@1.0.0" world. (This isn't a very realistic example
    /// because a fallback world would usually be imports-only.)
    fn target_import_only_forgiving(wit_path: &Path) -> TargetEnvironment {
        let candidate_worlds = load_simple_world(wit_path, "spin:test/simple-import-only@1.0.0");

        TargetEnvironment {
            name: "test".to_owned(),
            trigger_worlds: [].into_iter().collect(),
            trigger_capabilities: Default::default(),
            unknown_trigger: UnknownTrigger::Allow(candidate_worlds),
            unknown_capabilities: Default::default(),
        }
    }

    #[tokio::test]
    async fn can_validate_component() {
        let wit_path = PathBuf::from(SIMPLE_WIT_DIR);

        let wit_text = tokio::fs::read_to_string(wit_path.join("world.wit"))
            .await
            .unwrap();
        let wasm = generate_dummy_component(&wit_text, "spin:test/simple@1.0.0");

        let env = target_simple_world(&wit_path);

        assert!(env.supports_trigger_type(&"s".to_owned()));
        assert!(!env.supports_trigger_type(&"t".to_owned()));

        let component = crate::ComponentToValidate::new("scomp", "scomp.wasm", wasm, vec![]);
        let errs =
            crate::validate_component_against_environments(&[env], &"s".to_owned(), &component)
                .await;
        assert!(
            errs.is_empty(),
            "{}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[tokio::test]
    async fn can_validate_component_for_unknown_trigger() {
        let wit_path = PathBuf::from(SIMPLE_WIT_DIR);

        let wit_text = tokio::fs::read_to_string(wit_path.join("world.wit"))
            .await
            .unwrap();
        // The actual component has an export, although the target world can't check that
        let wasm = generate_dummy_component(&wit_text, "spin:test/simple@1.0.0");

        let env = target_import_only_forgiving(&wit_path);

        // E.g. a plugin trigger that isn't part of the Spin CLI
        let non_existent_trigger = "farmer-buckleys-trousers-explode".to_owned();

        assert!(env.supports_trigger_type(&non_existent_trigger));

        let component = crate::ComponentToValidate::new("comp", "comp.wasm", wasm, vec![]);
        let errs = crate::validate_component_against_environments(
            &[env],
            &non_existent_trigger,
            &component,
        )
        .await;
        assert!(
            errs.is_empty(),
            "{}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[tokio::test]
    async fn can_validate_component_with_host_requirement() {
        let wit_path = PathBuf::from(SIMPLE_WIT_DIR);

        let wit_text = tokio::fs::read_to_string(wit_path.join("world.wit"))
            .await
            .unwrap();
        let wasm = generate_dummy_component(&wit_text, "spin:test/simple@1.0.0");

        let mut env = target_simple_world(&wit_path);
        env.trigger_capabilities.insert(
            "s".to_owned(),
            vec![
                "local_spline_reticulation".to_owned(),
                "nice_cup_of_tea".to_owned(),
            ],
        );

        assert!(env.supports_trigger_type(&"s".to_owned()));
        assert!(!env.supports_trigger_type(&"t".to_owned()));

        let component = crate::ComponentToValidate::new(
            "cscomp",
            "cscomp.wasm",
            wasm,
            vec!["nice_cup_of_tea".to_string()],
        );
        let errs =
            crate::validate_component_against_environments(&[env], &"s".to_owned(), &component)
                .await;
        assert!(
            errs.is_empty(),
            "{}",
            errs.iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[tokio::test]
    async fn unavailable_import_invalidates_component() {
        let wit_path = PathBuf::from(SIMPLE_WIT_DIR);

        let wit_text = tokio::fs::read_to_string(wit_path.join("world.wit"))
            .await
            .unwrap();
        let wasm = generate_dummy_component(&wit_text, "spin:test/not-so-simple@1.0.0");

        let env = target_simple_world(&wit_path);

        let component = crate::ComponentToValidate::new("nscomp", "nscomp.wasm", wasm, vec![]);
        let errs =
            crate::validate_component_against_environments(&[env], &"s".to_owned(), &component)
                .await;
        assert!(!errs.is_empty());

        let err = errs[0].to_string();
        assert!(
            err.contains("Component nscomp (nscomp.wasm) can't run in environment test"),
            "unexpected error {err}"
        );
        assert!(err.contains(
            "world spin:test/simple@1.0.0 does not provide an import named spin:test/evil@1.0.0"
        ), "unexpected error {err}");
    }

    #[tokio::test]
    async fn unprovided_export_invalidates_component() {
        let wit_path = PathBuf::from(SIMPLE_WIT_DIR);

        let wit_text = tokio::fs::read_to_string(wit_path.join("world.wit"))
            .await
            .unwrap();
        let wasm = generate_dummy_component(&wit_text, "spin:test/too-darn-simple@1.0.0");

        let env = target_simple_world(&wit_path);

        let component = crate::ComponentToValidate::new("tdscomp", "tdscomp.wasm", wasm, vec![]);
        let errs =
            crate::validate_component_against_environments(&[env], &"s".to_owned(), &component)
                .await;
        assert!(!errs.is_empty());

        let err = errs[0].to_string();
        assert!(
            err.contains("Component tdscomp (tdscomp.wasm) can't run in environment test"),
            "unexpected error {err}"
        );
    }

    #[tokio::test]
    async fn unsupported_host_req_invalidates_component() {
        let wit_path = PathBuf::from(SIMPLE_WIT_DIR);

        let wit_text = tokio::fs::read_to_string(wit_path.join("world.wit"))
            .await
            .unwrap();
        let wasm = generate_dummy_component(&wit_text, "spin:test/simple@1.0.0");

        let env = target_simple_world(&wit_path);

        assert!(env.supports_trigger_type(&"s".to_owned()));
        assert!(!env.supports_trigger_type(&"t".to_owned()));

        let component = crate::ComponentToValidate::new(
            "cscomp",
            "cscomp.wasm",
            wasm,
            vec!["nice_cup_of_tea".to_string()],
        );
        let errs =
            crate::validate_component_against_environments(&[env], &"s".to_owned(), &component)
                .await;
        assert!(!errs.is_empty());

        let err = errs[0].to_string();
        assert!(
            err.contains("Component cscomp can't run in environment test"),
            "unexpected error {err}"
        );
        assert!(err.contains("nice_cup_of_tea"), "unexpected error {err}");
    }

    fn generate_dummy_component(wit: &str, world: &str) -> Vec<u8> {
        let mut resolve = wit_parser::Resolve::default();
        let package_id = resolve.push_str("test", wit).expect("should parse WIT");
        let world_id = resolve
            .select_world(package_id, Some(world))
            .expect("should select world");

        let mut wasm = wit_component::dummy_module(
            &resolve,
            world_id,
            wit_parser::ManglingAndAbi::Legacy(wit_parser::LiftLowerAbi::Sync),
        );
        wit_component::embed_component_metadata(
            &mut wasm,
            &resolve,
            world_id,
            wit_component::StringEncoding::UTF8,
        )
        .expect("should embed component metadata");

        let mut encoder = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&wasm)
            .expect("should set module");
        encoder.encode().expect("should encode component")
    }
}
