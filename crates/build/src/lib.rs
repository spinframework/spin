#![deny(missing_docs)]

//! A library for building Spin components.

mod manifest;

use anyhow::{anyhow, bail, Context, Result};
use manifest::ComponentBuildInfo;
use spin_common::{paths::parent_dir, ui::quoted_path};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use subprocess::{Exec, Redirection};

use crate::manifest::component_build_configs;

/// If present, run the build command of each component.
pub async fn build(
    manifest_file: &Path,
    component_ids: &[String],
    target_checks: TargetChecking,
    cache_root: Option<PathBuf>,
) -> Result<()> {
    let build_info = component_build_configs(manifest_file)
        .await
        .with_context(|| {
            format!(
                "Cannot read manifest file from {}",
                quoted_path(manifest_file)
            )
        })?;
    let app_dir = parent_dir(manifest_file)?;

    let build_result = build_components(component_ids, build_info.components(), &app_dir);

    // Emit any required warnings now, so that they don't bury any errors.
    if let Some(e) = build_info.load_error() {
        // The manifest had errors. We managed to attempt a build anyway, but we want to
        // let the user know about them.
        terminal::warn!("The manifest has errors not related to the Wasm component build. Error details:\n{e:#}");
        // Checking deployment targets requires a healthy manifest (because trigger types etc.),
        // if any of these were specified, warn they are being skipped.
        let should_have_checked_targets =
            target_checks.check() && build_info.has_deployment_targets();
        if should_have_checked_targets {
            terminal::warn!(
                "The manifest error(s) prevented Spin from checking the deployment targets."
            );
        }
    }

    // If the build failed, exit with an error at this point.
    build_result?;

    let Some(manifest) = build_info.manifest() else {
        // We can't proceed to checking (because that needs a full healthy manifest), and we've
        // already emitted any necessary warning, so quit.
        return Ok(());
    };

    if target_checks.check() {
        let application = spin_environments::ApplicationToValidate::new(
            manifest.clone(),
            manifest_file.parent().unwrap(),
        )
        .await
        .context("unable to load application for checking against deployment targets")?;
        let target_validation = spin_environments::validate_application_against_environment_ids(
            &application,
            build_info.deployment_targets(),
            cache_root.clone(),
            &app_dir,
        )
        .await
        .context("unable to check if the application is compatible with deployment targets")?;

        if !target_validation.is_ok() {
            for error in target_validation.errors() {
                terminal::error!("{error}");
            }
            anyhow::bail!("All components built successfully, but one or more was incompatible with one or more of the deployment targets.");
        }
    }

    Ok(())
}

/// Run all component build commands, using the default options (build all
/// components, perform target checking). We run a "default build" in several
/// places and this centralises the logic of what such a "default build" means.
pub async fn build_default(manifest_file: &Path, cache_root: Option<PathBuf>) -> Result<()> {
    build(manifest_file, &[], TargetChecking::Check, cache_root).await
}

fn build_components(
    component_ids: &[String],
    components: Vec<ComponentBuildInfo>,
    app_dir: &Path,
) -> Result<(), anyhow::Error> {
    let components_to_build = if component_ids.is_empty() {
        components
    } else {
        let all_ids: HashSet<_> = components.iter().map(|c| &c.id).collect();
        let unknown_component_ids: Vec<_> = component_ids
            .iter()
            .filter(|id| !all_ids.contains(id))
            .map(|s| s.as_str())
            .collect();

        if !unknown_component_ids.is_empty() {
            bail!("Unknown component(s) {}", unknown_component_ids.join(", "));
        }

        components
            .into_iter()
            .filter(|c| component_ids.contains(&c.id))
            .collect()
    };

    if components_to_build.iter().all(|c| c.build.is_none()) {
        println!("None of the components have a build command.");
        println!("For information on specifying a build command, see https://spinframework.dev/build#setting-up-for-spin-build.");
        return Ok(());
    }

    components_to_build
        .into_iter()
        .map(|c| build_component(c, app_dir))
        .collect::<Result<Vec<_>, _>>()?;

    terminal::step!("Finished", "building all Spin components");
    Ok(())
}

/// Run the build command of the component.
fn build_component(build_info: ComponentBuildInfo, app_dir: &Path) -> Result<()> {
    match build_info.build {
        Some(b) => {
            let command_count = b.commands().len();

            if command_count > 1 {
                terminal::step!(
                    "Building",
                    "component {} ({} commands)",
                    build_info.id,
                    command_count
                );
            }

            for (index, command) in b.commands().enumerate() {
                if command_count > 1 {
                    terminal::step!(
                        "Running build step",
                        "{}/{} for component {} with '{}'",
                        index + 1,
                        command_count,
                        build_info.id,
                        command
                    );
                } else {
                    terminal::step!("Building", "component {} with `{}`", build_info.id, command);
                }

                let workdir = construct_workdir(app_dir, b.workdir.as_ref())?;
                if b.workdir.is_some() {
                    println!("Working directory: {}", quoted_path(&workdir));
                }

                let exit_status = Exec::shell(command)
                    .cwd(workdir)
                    .stdout(Redirection::None)
                    .stderr(Redirection::None)
                    .stdin(Redirection::None)
                    .popen()
                    .map_err(|err| {
                        anyhow!(
                            "Cannot spawn build process '{:?}' for component {}: {}",
                            &b.command,
                            build_info.id,
                            err
                        )
                    })?
                    .wait()?;

                if !exit_status.success() {
                    bail!(
                        "Build command for component {} failed with status {:?}",
                        build_info.id,
                        exit_status,
                    );
                }
            }

            Ok(())
        }
        _ => Ok(()),
    }
}

/// Constructs the absolute working directory in which to run the build command.
fn construct_workdir(app_dir: &Path, workdir: Option<impl AsRef<Path>>) -> Result<PathBuf> {
    let mut cwd = app_dir.to_owned();

    if let Some(workdir) = workdir {
        // Using `Path::has_root` as `is_relative` and `is_absolute` have
        // surprising behavior on Windows, see:
        // https://doc.rust-lang.org/std/path/struct.Path.html#method.is_absolute
        if workdir.as_ref().has_root() {
            bail!("The workdir specified in the application file must be relative.");
        }
        cwd.push(workdir);
    }

    Ok(cwd)
}

/// Specifies target environment checking behaviour
pub enum TargetChecking {
    /// The build should check that all components are compatible with all target environments.
    Check,
    /// The build should not check target environments.
    Skip,
}

impl TargetChecking {
    /// Should the build check target environments?
    fn check(&self) -> bool {
        matches!(self, Self::Check)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_data_root() -> PathBuf {
        let crate_dir = env!("CARGO_MANIFEST_DIR");
        PathBuf::from(crate_dir).join("tests")
    }

    #[tokio::test]
    async fn can_load_even_if_trigger_invalid() {
        let bad_trigger_file = test_data_root().join("bad_trigger.toml");
        build(&bad_trigger_file, &[], TargetChecking::Skip, None)
            .await
            .unwrap();
    }
}
