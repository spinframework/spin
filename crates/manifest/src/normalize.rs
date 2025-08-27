//! Manifest normalization functions.

use std::collections::HashSet;

use crate::schema::v2::{AppManifest, ComponentSpec, KebabId};

/// Normalizes some optional [`AppManifest`] features into a canonical form:
/// - Inline components in trigger configs are moved into top-level
///   components and replaced with a reference.
/// - Any triggers without an ID are assigned a generated ID.
pub fn normalize_manifest(manifest: &mut AppManifest, profile: Option<&str>) {
    normalize_trigger_ids(manifest);
    normalize_inline_components(manifest);
    apply_profile_overrides(manifest, profile);
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
