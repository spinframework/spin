use anyhow::{anyhow, Context};

mod environment;
mod loader;

use environment::{CandidateWorld, CandidateWorlds, TargetEnvironment, TriggerType};
pub use loader::ApplicationToValidate;
use loader::ComponentToValidate;
use spin_manifest::schema::v2::TargetEnvironmentRef;

/// The result of validating an application against a list of target environments.
/// If `is_ok` returns true (or equivalently if the `errors` collection is empty),
/// the application passed validation, and can run in all the environments against
/// which it was checked. Otherwise, at least one component cannot run in at least
/// one target environment, and the `errors` collection contains the details.
#[derive(Default)]
pub struct TargetEnvironmentValidation(Vec<anyhow::Error>);

impl TargetEnvironmentValidation {
    pub fn is_ok(&self) -> bool {
        self.0.is_empty()
    }

    pub fn errors(&self) -> &[anyhow::Error] {
        &self.0
    }
}

/// Validates *all* application components against the list of referenced target enviroments. Each component must conform
/// to *all* environments to pass.
///
/// If the return value is `Ok(...)`, this means only that we were able to perform the validation.
/// The caller **MUST** still check the returned [TargetEnvironmentValidation] to determine the
/// outcome of validation.
///
/// If the return value is `Err(...)`, then we weren't able even to attempt validation.
pub async fn validate_application_against_environment_ids(
    application: &ApplicationToValidate,
    env_ids: &[TargetEnvironmentRef],
    cache_root: Option<std::path::PathBuf>,
    app_dir: &std::path::Path,
) -> anyhow::Result<TargetEnvironmentValidation> {
    if env_ids.is_empty() {
        return Ok(Default::default());
    }

    let envs = TargetEnvironment::load_all(env_ids, cache_root, app_dir).await?;
    validate_application_against_environments(application, &envs).await
}

/// Validates *all* application components against the list of (realised) target enviroments. Each component must conform
/// to *all* environments to pass.
///
/// For the slightly funky return type, see [validate_application_against_environment_ids].
async fn validate_application_against_environments(
    application: &ApplicationToValidate,
    envs: &[TargetEnvironment],
) -> anyhow::Result<TargetEnvironmentValidation> {
    for trigger_type in application.trigger_types() {
        if let Some(env) = envs.iter().find(|e| !e.supports_trigger_type(trigger_type)) {
            anyhow::bail!(
                "Environment {} does not support trigger type {trigger_type}",
                env.name()
            );
        }
    }

    let components_by_trigger_type = application.components_by_trigger_type().await?;

    let mut errs = vec![];

    for (trigger_type, component) in components_by_trigger_type {
        for component in &component {
            errs.extend(
                validate_component_against_environments(envs, &trigger_type, component).await,
            );
        }
    }

    Ok(TargetEnvironmentValidation(errs))
}

/// Validates the component against the list of target enviroments. The component must conform
/// to *all* environments to pass.
///
/// The return value contains the list of validation errors. There may be up to one error per
/// target environment, explaining why the component cannot run in that environment.
/// An empty list means the component has passed validation and is compatible with
/// all target environments.
async fn validate_component_against_environments(
    envs: &[TargetEnvironment],
    trigger_type: &TriggerType,
    component: &ComponentToValidate<'_>,
) -> Vec<anyhow::Error> {
    let mut errs = vec![];

    for env in envs {
        let worlds = env.worlds(trigger_type);
        if let Some(e) = validate_wasm_against_any_world(env, worlds, component)
            .await
            .err()
        {
            errs.push(e);
        }

        let host_caps = env.capabilities(trigger_type);
        if let Some(e) = validate_host_reqs(env, host_caps, component).err() {
            errs.push(e);
        }
    }

    if errs.is_empty() {
        tracing::info!(
            "Validated component {} {} against all target worlds",
            component.id(),
            component.source_description()
        );
    }

    errs
}

/// Validates the component against the list of candidate worlds. The component must conform
/// to *at least one* candidate world to pass (since if it can run in one world provided by
/// the target environment, it can run in the target environment).
async fn validate_wasm_against_any_world(
    env: &TargetEnvironment,
    worlds: &CandidateWorlds,
    component: &ComponentToValidate<'_>,
) -> anyhow::Result<()> {
    let mut result = Ok(());
    for target_world in worlds {
        tracing::debug!(
            "Trying component {} {} against target world {target_world}",
            component.id(),
            component.source_description(),
        );
        match validate_wasm_against_world(env, target_world, component).await {
            Ok(()) => {
                tracing::info!(
                    "Validated component {} {} against target world {target_world}",
                    component.id(),
                    component.source_description(),
                );
                return Ok(());
            }
            Err(e) => {
                // Record the error, but continue in case a different world succeeds
                tracing::info!(
                    "Rejecting component {} {} for target world {target_world} because {e:?}",
                    component.id(),
                    component.source_description(),
                );
                result = Err(e);
            }
        }
    }
    result
}

async fn validate_wasm_against_world(
    env: &TargetEnvironment,
    target_world: &CandidateWorld,
    component: &ComponentToValidate<'_>,
) -> anyhow::Result<()> {
    use wac_types::{validate_target, ItemKind, Package as WacPackage, Types as WacTypes, WorldId};

    // Gets the selected world from the component encoded WIT package
    // TODO: make this an export on `wac_types::Types`.
    fn get_wit_world(
        types: &WacTypes,
        top_level_world: WorldId,
        world_name: &str,
    ) -> anyhow::Result<WorldId> {
        let top_level_world = &types[top_level_world];
        let world = top_level_world
            .exports
            .get(world_name)
            .with_context(|| format!("wit package did not contain a world named '{world_name}'"))?;

        let ItemKind::Type(wac_types::Type::World(world_id)) = world else {
            // We expect the top-level world to export a world type
            anyhow::bail!("wit package was not encoded properly")
        };
        let wit_world = &types[*world_id];
        let world = wit_world.exports.values().next();
        let Some(ItemKind::Component(w)) = world else {
            // We expect the nested world type to export a component
            anyhow::bail!("wit package was not encoded properly")
        };
        Ok(*w)
    }

    let mut types = WacTypes::default();

    let target_world_package = WacPackage::from_bytes(
        &target_world.package_namespaced_name(),
        target_world.package_version(),
        target_world.package_bytes(),
        &mut types,
    )?;

    let target_world_id =
        get_wit_world(&types, target_world_package.ty(), target_world.world_name())?;

    let component_package =
        WacPackage::from_bytes(component.id(), None, component.wasm_bytes(), &mut types)?;

    let target_result = validate_target(&types, target_world_id, component_package.ty());

    match target_result {
        Ok(_) => Ok(()),
        Err(report) => Err(format_target_result_error(
            &types,
            env.name(),
            target_world.to_string(),
            component.id(),
            component.source_description(),
            &report,
        )),
    }
}

fn validate_host_reqs(
    env: &TargetEnvironment,
    host_caps: &[String],
    component: &ComponentToValidate,
) -> anyhow::Result<()> {
    let unsatisfied: Vec<_> = component
        .host_requirements()
        .iter()
        .filter(|host_req| !satisfies(host_caps, host_req))
        .cloned()
        .collect();
    if unsatisfied.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("Component {} can't run in environment {} because it requires the feature(s) '{}' which the environment does not support", component.id(), env.name(), unsatisfied.join(", ")))
    }
}

fn satisfies(host_caps: &[String], host_req: &String) -> bool {
    host_caps.contains(host_req)
}

fn format_target_result_error(
    types: &wac_types::Types,
    env_name: &str,
    target_world_name: String,
    component_id: &str,
    source_description: &str,
    report: &wac_types::TargetValidationReport,
) -> anyhow::Error {
    let mut error_string = format!(
        "Component {} ({}) can't run in environment {} because world {} ...\n",
        component_id, source_description, env_name, target_world_name
    );

    for (idx, import) in report.imports_not_in_target().enumerate() {
        if idx == 0 {
            error_string.push_str("... requires imports named\n  - ");
        } else {
            error_string.push_str("  - ");
        }
        error_string.push_str(import);
        error_string.push('\n');
    }

    for (idx, (export, export_kind)) in report.missing_exports().enumerate() {
        if idx == 0 {
            error_string.push_str("... requires exports named\n  - ");
        } else {
            error_string.push_str("  - ");
        }
        error_string.push_str(export);
        error_string.push_str(" (");
        error_string.push_str(export_kind.desc(types));
        error_string.push_str(")\n");
    }

    for (name, extern_kind, error) in report.mismatched_types() {
        error_string.push_str("... found a type mismatch for ");
        error_string.push_str(&format!("{extern_kind} {name}: {error}"));
    }

    anyhow!(error_string)
}
