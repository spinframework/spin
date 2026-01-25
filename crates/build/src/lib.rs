#![deny(missing_docs)]

//! A library for building Spin components.

mod manifest;

use anyhow::{anyhow, bail, Context, Result};
use manifest::ComponentBuildInfo;
use spin_common::{paths::parent_dir, ui::quoted_path};
use spin_manifest::schema::v2;
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
    wit_generation: GenerateDependencyWits,
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

    let components_to_build = components_to_build(component_ids, build_info.components())?;

    if wit_generation.generate() {
        regenerate_wits(&components_to_build, &app_dir).await?;
    }

    let build_result = build_components(components_to_build, &app_dir);

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
    build(
        manifest_file,
        &[],
        TargetChecking::Check,
        GenerateDependencyWits::Generate,
        cache_root,
    )
    .await
}

fn components_to_build(
    component_ids: &[String],
    components: Vec<ComponentBuildInfo>,
) -> anyhow::Result<Vec<ComponentBuildInfo>> {
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

    Ok(components_to_build)
}

async fn regenerate_wits(
    components_to_build: &[ComponentBuildInfo],
    app_root: &Path,
) -> anyhow::Result<()> {
    for component in components_to_build {
        let component_dir = match component.build.as_ref().and_then(|b| b.workdir.as_ref()) {
            None => app_root.to_owned(),
            Some(d) => app_root.join(d),
        };
        let dest_file = component_dir.join("spin-dependencies.wit");
        spin_dependency_wit::extract_wits_into(
            component.dependencies.inner.iter(),
            app_root,
            dest_file,
        )
        .await?;
    }

    Ok(())
}

fn build_components(
    components_to_build: Vec<ComponentBuildInfo>,
    app_dir: &Path,
) -> anyhow::Result<()> {
    if components_to_build.iter().all(|c| c.build.is_none()) {
        println!("None of the components have a build command.");
        println!("For information on specifying a build command, see https://spinframework.dev/build#setting-up-for-spin-build.");
        return Ok(());
    }

    // If dependencies are being built as part of `spin build`, we would like
    // them to be rebuilt earlier (e.g. so that consumers using the binary as a source
    // of type information see the latest interface).
    let (components_to_build, has_cycle) = sort(components_to_build);

    if has_cycle {
        tracing::debug!("There is a dependency cycle among components. Spin cannot guarantee to build dependencies before consumers.");
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

#[derive(Clone)]
struct SortableBuildInfo {
    source: Option<String>,
    local_dependency_paths: Vec<String>,
    build_info: ComponentBuildInfo,
}

impl From<&ComponentBuildInfo> for SortableBuildInfo {
    fn from(value: &ComponentBuildInfo) -> Self {
        fn local_dep_path(dep: &v2::ComponentDependency) -> Option<String> {
            match dep {
                v2::ComponentDependency::Local { path, .. } => Some(path.display().to_string()),
                _ => None,
            }
        }

        let source = match value.source.as_ref() {
            Some(spin_manifest::schema::v2::ComponentSource::Local(path)) => Some(path.clone()),
            _ => None,
        };
        let local_dependency_paths = value
            .dependencies
            .inner
            .values()
            .filter_map(local_dep_path)
            .collect();

        Self {
            source,
            local_dependency_paths,
            build_info: value.clone(),
        }
    }
}

impl std::hash::Hash for SortableBuildInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.build_info.id.hash(state);
        self.source.hash(state);
        self.local_dependency_paths.hash(state);
    }
}

impl PartialEq for SortableBuildInfo {
    fn eq(&self, other: &Self) -> bool {
        self.build_info.id == other.build_info.id
            && self.source == other.source
            && self.local_dependency_paths == other.local_dependency_paths
    }
}

impl Eq for SortableBuildInfo {}

/// Topo sort by local path dependency. Second result is if there was a cycle.
fn sort(components: Vec<ComponentBuildInfo>) -> (Vec<ComponentBuildInfo>, bool) {
    let sortables = components
        .iter()
        .map(SortableBuildInfo::from)
        .collect::<Vec<_>>();
    let mut sorter = topological_sort::TopologicalSort::<SortableBuildInfo>::new();

    for s in &sortables {
        sorter.insert(s.clone());
    }

    for s1 in &sortables {
        for dep in &s1.local_dependency_paths {
            for s2 in &sortables {
                if s2.source.as_ref().is_some_and(|src| src == dep) {
                    // s1 depends on s2
                    sorter.add_link(topological_sort::DependencyLink {
                        prec: s2.clone(),
                        succ: s1.clone(),
                    });
                }
            }
        }
    }

    let result = sorter.map(|s| s.build_info).collect::<Vec<_>>();

    // We shouldn't refuse to build if a cycle occurs, so return the original order to allow
    // stuff to proceed.  (We could be smarter about this, but really it's a pathological situation
    // and we don't need to bust a gut over it.)
    if result.len() == components.len() {
        (result, false)
    } else {
        (components, true)
    }
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

/// Specifies dependency WIT generation behaviour
pub enum GenerateDependencyWits {
    /// The build should generate WITs for component dependencies.
    Generate,
    /// The build should not generate WITs.
    Skip,
}

impl GenerateDependencyWits {
    /// Should the build generate dependency WITs?
    fn generate(&self) -> bool {
        matches!(self, Self::Generate)
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
        build(
            &bad_trigger_file,
            &[],
            TargetChecking::Skip,
            GenerateDependencyWits::Skip,
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn succeeds_if_target_env_matches() {
        let manifest_path = test_data_root().join("good_target_env.toml");
        build(
            &manifest_path,
            &[],
            TargetChecking::Check,
            GenerateDependencyWits::Skip,
            None,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn fails_if_target_env_does_not_match() {
        let manifest_path = test_data_root().join("bad_target_env.toml");
        let err = build(
            &manifest_path,
            &[],
            TargetChecking::Check,
            GenerateDependencyWits::Skip,
            None,
        )
        .await
        .expect_err("should have failed")
        .to_string();

        // build prints validation errors rather than returning them to top level
        // (because there could be multiple errors) - see has_meaningful_error_if_target_env_does_not_match
        assert!(
            err.contains("one or more was incompatible with one or more of the deployment targets")
        );
    }

    #[tokio::test]
    async fn has_meaningful_error_if_target_env_does_not_match() {
        let manifest_file = test_data_root().join("bad_target_env.toml");
        let manifest = spin_manifest::manifest_from_file(&manifest_file).unwrap();
        let application = spin_environments::ApplicationToValidate::new(
            manifest.clone(),
            manifest_file.parent().unwrap(),
        )
        .await
        .context("unable to load application for checking against deployment targets")
        .unwrap();

        let target_validation = spin_environments::validate_application_against_environment_ids(
            &application,
            spin_environments::Targets {
                default: &manifest.application.targets,
                overrides: std::collections::HashMap::new(),
            },
            None,
            manifest_file.parent().unwrap(),
        )
        .await
        .context("unable to check if the application is compatible with deployment targets")
        .unwrap();

        assert_eq!(1, target_validation.errors().len());

        let err = target_validation.errors()[0].to_string();

        assert!(err.contains("can't run in environment wasi-minimal"));
        assert!(err.contains("world wasi:cli/command@0.2.0"));
        assert!(err.contains("requires imports named"));
        assert!(err.contains("wasi:cli/stdout"));
    }

    fn dummy_buildinfo(id: &str) -> ComponentBuildInfo {
        dummy_build_info_deps(id, &[])
    }

    fn dummy_build_info_dep(id: &str, dep_on: &str) -> ComponentBuildInfo {
        dummy_build_info_deps(id, &[dep_on])
    }

    fn dummy_build_info_deps(id: &str, dep_on: &[&str]) -> ComponentBuildInfo {
        ComponentBuildInfo {
            id: id.into(),
            source: Some(v2::ComponentSource::Local(format!("{id}.wasm"))),
            build: None,
            dependencies: depends_on(dep_on),
            targets: None,
        }
    }

    fn depends_on(paths: &[&str]) -> v2::ComponentDependencies {
        let mut deps = vec![];
        for (index, path) in paths.iter().enumerate() {
            let dep_name =
                spin_serde::DependencyName::Plain(format!("dummy{index}").try_into().unwrap());
            let dep = v2::ComponentDependency::Local {
                path: path.into(),
                export: None,
            };
            deps.push((dep_name, dep));
        }
        v2::ComponentDependencies {
            inner: deps.into_iter().collect(),
        }
    }

    /// Asserts that id `before` comes before id `after` in collection `cs`
    fn assert_before(cs: &[ComponentBuildInfo], before: &str, after: &str) {
        assert!(
            cs.iter().position(|c| c.id == before).unwrap()
                < cs.iter().position(|c| c.id == after).unwrap()
        );
    }

    #[test]
    fn if_no_dependencies_then_all_build() {
        let (cs, had_cycle) = sort(vec![dummy_buildinfo("1"), dummy_buildinfo("2")]);
        assert_eq!(2, cs.len());
        assert!(cs.iter().any(|c| c.id == "1"));
        assert!(cs.iter().any(|c| c.id == "2"));
        assert!(!had_cycle);
    }

    #[test]
    fn dependencies_build_before_consumers() {
        let (cs, had_cycle) = sort(vec![
            dummy_buildinfo("1"),
            dummy_build_info_dep("2", "3.wasm"),
            dummy_buildinfo("3"),
            dummy_build_info_dep("4", "1.wasm"),
        ]);
        assert_eq!(4, cs.len());
        assert_before(&cs, "1", "4");
        assert_before(&cs, "3", "2");
        assert!(!had_cycle);
    }

    #[test]
    fn multiple_dependencies_build_before_consumers() {
        let (cs, had_cycle) = sort(vec![
            dummy_buildinfo("1"),
            dummy_build_info_dep("2", "3.wasm"),
            dummy_buildinfo("3"),
            dummy_build_info_dep("4", "1.wasm"),
            dummy_build_info_dep("5", "3.wasm"),
            dummy_build_info_deps("6", &["3.wasm", "2.wasm"]),
            dummy_buildinfo("7"),
        ]);
        assert_eq!(7, cs.len());
        assert_before(&cs, "1", "4");
        assert_before(&cs, "3", "2");
        assert_before(&cs, "3", "5");
        assert_before(&cs, "3", "6");
        assert_before(&cs, "2", "6");
        assert!(!had_cycle);
    }

    #[test]
    fn circular_dependencies_dont_prevent_build() {
        let (cs, had_cycle) = sort(vec![
            dummy_buildinfo("1"),
            dummy_build_info_dep("2", "3.wasm"),
            dummy_build_info_dep("3", "2.wasm"),
            dummy_build_info_dep("4", "1.wasm"),
        ]);
        assert_eq!(4, cs.len());
        assert!(cs.iter().any(|c| c.id == "1"));
        assert!(cs.iter().any(|c| c.id == "2"));
        assert!(cs.iter().any(|c| c.id == "3"));
        assert!(cs.iter().any(|c| c.id == "4"));
        assert!(had_cycle);
    }

    #[test]
    fn non_path_dependencies_do_not_prevent_sorting() {
        let mut depends_on_remote = dummy_buildinfo("2");
        depends_on_remote.dependencies.inner.insert(
            spin_serde::DependencyName::Plain("remote".to_owned().try_into().unwrap()),
            v2::ComponentDependency::Version("1.2.3".to_owned()),
        );

        let mut depends_on_local_and_remote = dummy_build_info_dep("4", "1.wasm");
        depends_on_local_and_remote.dependencies.inner.insert(
            spin_serde::DependencyName::Plain("remote".to_owned().try_into().unwrap()),
            v2::ComponentDependency::Version("1.2.3".to_owned()),
        );

        let (cs, _) = sort(vec![
            dummy_buildinfo("1"),
            depends_on_remote,
            dummy_buildinfo("3"),
            depends_on_local_and_remote,
        ]);

        assert_eq!(4, cs.len());
        assert_before(&cs, "1", "4");
    }

    #[test]
    fn non_path_sources_do_not_prevent_sorting() {
        let mut remote_source = dummy_build_info_dep("2", "3.wasm");
        remote_source.source = Some(v2::ComponentSource::Remote {
            url: "far://away".into(),
            digest: "loadsa-hex".into(),
        });

        let (cs, _) = sort(vec![
            dummy_buildinfo("1"),
            remote_source,
            dummy_buildinfo("3"),
            dummy_build_info_dep("4", "1.wasm"),
        ]);

        assert_eq!(4, cs.len());
        assert_before(&cs, "1", "4");
    }

    #[test]
    fn dependencies_on_non_manifest_components_do_not_prevent_sorting() {
        let (cs, had_cycle) = sort(vec![
            dummy_buildinfo("1"),
            dummy_build_info_deps("2", &["3.wasm", "crikey.wasm"]),
            dummy_buildinfo("3"),
            dummy_build_info_dep("4", "1.wasm"),
        ]);
        assert_eq!(4, cs.len());
        assert_before(&cs, "1", "4");
        assert_before(&cs, "3", "2");
        assert!(!had_cycle);
    }
}
