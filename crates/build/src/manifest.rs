use anyhow::Result;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

use spin_manifest::{schema::v2, ManifestVersion};

#[allow(clippy::large_enum_variant)] // only ever constructed once
pub enum ManifestBuildInfo {
    Loadable {
        components: Vec<ComponentBuildInfo>,
        deployment_targets: Vec<spin_manifest::schema::v2::TargetEnvironmentRef>,
        manifest: spin_manifest::schema::v2::AppManifest,
    },
    Unloadable {
        components: Vec<ComponentBuildInfo>,
        has_deployment_targets: bool,
        load_error: spin_manifest::Error,
    },
}

impl ManifestBuildInfo {
    pub fn components(&self) -> Vec<ComponentBuildInfo> {
        match self {
            Self::Loadable { components, .. } => components.clone(),
            Self::Unloadable { components, .. } => components.clone(),
        }
    }

    pub fn load_error(&self) -> Option<&spin_manifest::Error> {
        match self {
            Self::Loadable { .. } => None,
            Self::Unloadable { load_error, .. } => Some(load_error),
        }
    }

    pub fn deployment_targets(&self) -> &[spin_manifest::schema::v2::TargetEnvironmentRef] {
        match self {
            Self::Loadable {
                deployment_targets, ..
            } => deployment_targets,
            Self::Unloadable { .. } => &[],
        }
    }

    pub fn has_deployment_targets(&self) -> bool {
        match self {
            Self::Loadable {
                deployment_targets, ..
            } => !deployment_targets.is_empty(),
            Self::Unloadable {
                has_deployment_targets,
                ..
            } => *has_deployment_targets,
        }
    }

    pub fn manifest(&self) -> Option<&spin_manifest::schema::v2::AppManifest> {
        match self {
            Self::Loadable { manifest, .. } => Some(manifest),
            Self::Unloadable { .. } => None,
        }
    }
}

/// Returns a map of component IDs to [`v2::ComponentBuildConfig`]s for the
/// given (v1 or v2) manifest path. If the manifest cannot be loaded, the
/// function attempts fallback: if fallback succeeds, result is Ok but the load error
/// is also returned via the second part of the return value tuple.
pub async fn component_build_configs(manifest_file: impl AsRef<Path>) -> Result<ManifestBuildInfo> {
    let manifest = spin_manifest::manifest_from_file(&manifest_file);
    match manifest {
        Ok(mut manifest) => {
            spin_manifest::normalize::normalize_manifest(&mut manifest);
            let components = build_configs_from_manifest(&manifest);
            let deployment_targets = deployment_targets_from_manifest(&manifest);
            Ok(ManifestBuildInfo::Loadable {
                components,
                deployment_targets,
                manifest,
            })
        }
        Err(load_error) => {
            // The manifest didn't load, but the problem might not be build-affecting.
            // Try to fall back by parsing out only the bits we need. And if something
            // goes wrong with the fallback, give up and return the original manifest load
            // error.
            let Ok(components) = fallback_load_build_configs(&manifest_file).await else {
                return Err(load_error.into());
            };
            let Ok(has_deployment_targets) = has_deployment_targets(&manifest_file).await else {
                return Err(load_error.into());
            };
            Ok(ManifestBuildInfo::Unloadable {
                components,
                has_deployment_targets,
                load_error,
            })
        }
    }
}

fn build_configs_from_manifest(
    manifest: &spin_manifest::schema::v2::AppManifest,
) -> Vec<ComponentBuildInfo> {
    manifest
        .components
        .iter()
        .map(|(id, c)| ComponentBuildInfo {
            id: id.to_string(),
            build: c.build.clone(),
        })
        .collect()
}

fn deployment_targets_from_manifest(
    manifest: &spin_manifest::schema::v2::AppManifest,
) -> Vec<spin_manifest::schema::v2::TargetEnvironmentRef> {
    manifest.application.targets.clone()
}

async fn fallback_load_build_configs(
    manifest_file: impl AsRef<Path>,
) -> Result<Vec<ComponentBuildInfo>> {
    let manifest_text = tokio::fs::read_to_string(manifest_file).await?;
    Ok(match ManifestVersion::detect(&manifest_text)? {
        ManifestVersion::V1 => {
            let v1: ManifestV1BuildInfo = toml::from_str(&manifest_text)?;
            v1.components
        }
        ManifestVersion::V2 => {
            let v2: ManifestV2BuildInfo = toml::from_str(&manifest_text)?;
            v2.components
                .into_iter()
                .map(|(id, mut c)| {
                    c.id = id;
                    c
                })
                .collect()
        }
    })
}

async fn has_deployment_targets(manifest_file: impl AsRef<Path>) -> Result<bool> {
    let manifest_text = tokio::fs::read_to_string(manifest_file).await?;
    Ok(match ManifestVersion::detect(&manifest_text)? {
        ManifestVersion::V1 => false,
        ManifestVersion::V2 => {
            let table: toml::value::Table = toml::from_str(&manifest_text)?;
            table
                .get("application")
                .and_then(|a| a.as_table())
                .and_then(|t| t.get("targets"))
                .and_then(|arr| arr.as_array())
                .is_some_and(|arr| !arr.is_empty())
        }
    })
}

#[derive(Clone, Deserialize)]
pub struct ComponentBuildInfo {
    #[serde(default)]
    pub id: String,
    pub build: Option<v2::ComponentBuildConfig>,
}

#[derive(Deserialize)]
struct ManifestV1BuildInfo {
    #[serde(rename = "component")]
    components: Vec<ComponentBuildInfo>,
}

#[derive(Deserialize)]
struct ManifestV2BuildInfo {
    #[serde(rename = "component")]
    components: BTreeMap<String, ComponentBuildInfo>,
}
