//! Manifest normalization functions.

use std::{collections::HashSet, path::PathBuf};

use crate::schema::v2::{AppManifest, ComponentSpec, KebabId};
use anyhow::Context;

/// Normalizes some optional [`AppManifest`] features into a canonical form:
/// - Inline components in trigger configs are moved into top-level
///   components and replaced with a reference.
/// - Any triggers without an ID are assigned a generated ID.
pub fn normalize_manifest(manifest: &mut AppManifest, profile: Option<&str>) -> anyhow::Result<()> {
    normalize_trigger_ids(manifest);
    normalize_inline_components(manifest);
    apply_profile_overrides(manifest, profile);
    normalize_dependency_component_refs(manifest)?;
    Ok(())
}

fn normalize_inline_components(manifest: &mut AppManifest) {
    // Normalize inline components
    let components = &mut manifest.components;

    for trigger in manifest.triggers.values_mut().flatten() {
        let trigger_id = &trigger.id;

        let component_specs = trigger
            .component
            .iter_mut()
            .chain(
                trigger
                    .components
                    .values_mut()
                    .flat_map(|specs| specs.0.iter_mut()),
            )
            .collect::<Vec<_>>();
        let multiple_components = component_specs.len() > 1;

        let mut counter = 1;
        for spec in component_specs {
            if !matches!(spec, ComponentSpec::Inline(_)) {
                continue;
            };

            let inline_id = {
                // Try a "natural" component ID...
                let mut id = KebabId::try_from(format!("{trigger_id}-component"));
                // ...falling back to a counter-based component ID
                if multiple_components
                    || id.is_err()
                    || components.contains_key(id.as_ref().unwrap())
                {
                    id = Ok(loop {
                        let id = KebabId::try_from(format!("inline-component{counter}")).unwrap();
                        if !components.contains_key(&id) {
                            break id;
                        }
                        counter += 1;
                    });
                }
                id.unwrap()
            };

            // Replace the inline component with a reference...
            let inline_spec = std::mem::replace(spec, ComponentSpec::Reference(inline_id.clone()));
            let ComponentSpec::Inline(component) = inline_spec else {
                unreachable!();
            };
            // ...moving the inline component into the top-level components map.
            components.insert(inline_id.clone(), *component);
        }
    }
}

fn normalize_trigger_ids(manifest: &mut AppManifest) {
    let mut trigger_ids = manifest
        .triggers
        .values()
        .flatten()
        .cloned()
        .map(|t| t.id)
        .collect::<HashSet<_>>();
    for (trigger_type, triggers) in &mut manifest.triggers {
        let mut counter = 1;
        for trigger in triggers {
            if !trigger.id.is_empty() {
                continue;
            }
            // Try to assign a "natural" ID to this trigger
            if let Some(ComponentSpec::Reference(component_id)) = &trigger.component {
                let candidate_id = format!("{component_id}-{trigger_type}-trigger");
                if !trigger_ids.contains(&candidate_id) {
                    trigger.id.clone_from(&candidate_id);
                    trigger_ids.insert(candidate_id);
                    continue;
                }
            }
            // Fall back to assigning a counter-based trigger ID
            trigger.id = loop {
                let id = format!("{trigger_type}-trigger{counter}");
                if !trigger_ids.contains(&id) {
                    trigger_ids.insert(id.clone());
                    break id;
                }
                counter += 1;
            }
        }
    }
}

fn apply_profile_overrides(manifest: &mut AppManifest, profile: Option<&str>) {
    let Some(profile) = profile else {
        return;
    };

    for (_, component) in &mut manifest.components {
        let Some(overrides) = component.profile.get(profile) else {
            continue;
        };

        if let Some(profile_build) = overrides.build.as_ref() {
            match component.build.as_mut() {
                None => {
                    component.build = Some(crate::schema::v2::ComponentBuildConfig {
                        command: profile_build.command.clone(),
                        workdir: None,
                        watch: vec![],
                    })
                }
                Some(build) => {
                    build.command = profile_build.command.clone();
                }
            }
        }

        if let Some(source) = overrides.source.as_ref() {
            component.source = source.clone();
        }

        component.environment.extend(overrides.environment.clone());

        component
            .dependencies
            .inner
            .extend(overrides.dependencies.inner.clone());
    }
}

use crate::schema::v2::{Component, ComponentDependency, ComponentSource};

fn normalize_dependency_component_refs(manifest: &mut AppManifest) -> anyhow::Result<()> {
    // `clone` a snapshot, because we are about to mutate collection elements,
    // and the borrow checker gets mad at us if we try to index into the collection
    // while that's happening.
    let components = manifest.components.clone();

    for (depender_id, component) in &mut manifest.components {
        for dependency in component.dependencies.inner.values_mut() {
            if let ComponentDependency::AppComponent {
                component: depended_on_id,
                export,
            } = dependency
            {
                let depended_on = components
                    .get(depended_on_id)
                    .with_context(|| format!("dependency ID {depended_on_id} does not exist"))?;
                ensure_is_acceptable_dependency(depended_on, depended_on_id, depender_id)?;
                *dependency = component_source_to_dependency(&depended_on.source, export.clone());
            }
        }
    }

    Ok(())
}

fn component_source_to_dependency(
    source: &ComponentSource,
    export: Option<String>,
) -> ComponentDependency {
    match source {
        ComponentSource::Local(path) => ComponentDependency::Local {
            path: PathBuf::from(path),
            export,
        },
        ComponentSource::Remote { url, digest } => ComponentDependency::HTTP {
            url: url.clone(),
            digest: digest.clone(),
            export,
        },
        ComponentSource::Registry {
            registry,
            package,
            version,
        } => ComponentDependency::Package {
            version: version.clone(),
            registry: registry.as_ref().map(|r| r.to_string()),
            package: Some(package.to_string()),
            export,
        },
    }
}

/// If a dependency has things like files or KV stores or network access...
/// those won't apply when it's composed, and that's likely to be surprising,
/// and developers hate surprises.
fn ensure_is_acceptable_dependency(
    component: &Component,
    depended_on_id: &KebabId,
    depender_id: &KebabId,
) -> anyhow::Result<()> {
    let mut surprises = vec![];

    // Explicitly discard fields we don't need to check (do *not* .. them away). This
    // way, the compiler will give us a heads up if a new field is added so we can
    // decide whether or not we need to check it.
    #[allow(deprecated)]
    let Component {
        source: _,
        description: _,
        variables,
        environment,
        files,
        exclude_files: _,
        allowed_http_hosts,
        allowed_outbound_hosts,
        key_value_stores,
        sqlite_databases,
        ai_models,
        build: _,
        tool: _,
        dependencies_inherit_configuration: _,
        dependencies,
        profile: _,
    } = component;

    if !ai_models.is_empty() {
        surprises.push("ai_models");
    }
    if !allowed_http_hosts.is_empty() {
        surprises.push("allowed_http_hosts");
    }
    if !allowed_outbound_hosts.is_empty() {
        surprises.push("allowed_outbound_hosts");
    }
    if !dependencies.inner.is_empty() {
        surprises.push("dependencies");
    }
    if !environment.is_empty() {
        surprises.push("environment");
    }
    if !files.is_empty() {
        surprises.push("files");
    }
    if !key_value_stores.is_empty() {
        surprises.push("key_value_stores");
    }
    if !sqlite_databases.is_empty() {
        surprises.push("sqlite_databases");
    }
    if !variables.is_empty() {
        surprises.push("variables");
    }

    if surprises.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("Dependencies may not have their own resources or permissions. Component {depended_on_id} cannot be used as a dependency of {depender_id} because it specifies: {}", surprises.join(", "));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use serde::Deserialize;
    use toml::toml;

    fn package_name(name: &str) -> spin_serde::DependencyName {
        let dpn = spin_serde::DependencyPackageName::try_from(name.to_string()).unwrap();
        spin_serde::DependencyName::Package(dpn)
    }

    #[test]
    fn can_resolve_dependency_on_file_source() {
        let mut manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2

            [application]
            name = "dummy"

            [[trigger.dummy]]
            component = "a"

            [component.a]
            source = "a.wasm"
            [component.a.dependencies]
            "b:b" = { component = "b" }

            [component.b]
            source = "b.wasm"
        })
        .unwrap();

        normalize_manifest(&mut manifest, None).unwrap();

        let dep = manifest
            .components
            .get("a")
            .unwrap()
            .dependencies
            .inner
            .get(&package_name("b:b"))
            .unwrap();

        let ComponentDependency::Local { path, export } = dep else {
            panic!("should have normalised to local dep");
        };

        assert_eq!(&PathBuf::from("b.wasm"), path);
        assert_eq!(&None, export);
    }

    #[test]
    fn can_resolve_dependency_on_http_source() {
        let mut manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2

            [application]
            name = "dummy"

            [[trigger.dummy]]
            component = "a"

            [component.a]
            source = "a.wasm"
            [component.a.dependencies]
            "b:b" = { component = "b", export = "c:d/e" }

            [component.b]
            source = { url = "http://example.com/b.wasm", digest = "12345" }
        })
        .unwrap();

        normalize_manifest(&mut manifest, None).unwrap();

        let dep = manifest
            .components
            .get("a")
            .unwrap()
            .dependencies
            .inner
            .get(&package_name("b:b"))
            .unwrap();

        let ComponentDependency::HTTP {
            url,
            digest,
            export,
        } = dep
        else {
            panic!("should have normalised to HTTP dep");
        };

        assert_eq!("http://example.com/b.wasm", url);
        assert_eq!("12345", digest);
        assert_eq!("c:d/e", export.as_ref().unwrap());
    }

    #[test]
    fn can_resolve_dependency_on_package() {
        let mut manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2

            [application]
            name = "dummy"

            [[trigger.dummy]]
            component = "a"

            [component.a]
            source = "a.wasm"
            [component.a.dependencies]
            "b:b" = { component = "b" }

            [component.b]
            source = { package = "bb:bb", version = "1.2.3", registry = "reginalds-registry.reg" }
        })
        .unwrap();

        normalize_manifest(&mut manifest, None).unwrap();

        let dep = manifest
            .components
            .get("a")
            .unwrap()
            .dependencies
            .inner
            .get(&package_name("b:b"))
            .unwrap();

        let ComponentDependency::Package {
            version,
            registry,
            package,
            export,
        } = dep
        else {
            panic!("should have normalised to HTTP dep");
        };

        assert_eq!("1.2.3", version);
        assert_eq!("reginalds-registry.reg", registry.as_ref().unwrap());
        assert_eq!("bb:bb", package.as_ref().unwrap());
        assert_eq!(&None, export);
    }
}
