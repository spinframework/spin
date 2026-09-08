//! Configuration of an application for the Spin runtime.

#![deny(missing_docs)]

pub mod compat;
pub mod error;
pub mod normalize;
pub mod schema;

use std::path::Path;

use schema::v2::AppManifest;

pub use error::Error;

/// Parses a V1 or V2 app manifest file into a [`AppManifest`].
pub fn manifest_from_file(path: impl AsRef<Path>) -> Result<AppManifest, Error> {
    let manifest_str = std::fs::read_to_string(path)?;
    manifest_from_str(&manifest_str)
}

/// Parses a V1 or V2 app manifest into a [`AppManifest`].
pub fn manifest_from_str(v1_or_v2_toml: &str) -> Result<AppManifest, Error> {
    // TODO: would it be faster to parse into a toml::Table rather than parse twice?
    match ManifestVersion::detect(v1_or_v2_toml)? {
        ManifestVersion::V1 => {
            let deserialized_v1 = toml::from_str(v1_or_v2_toml)?;
            compat::v1_to_v2_app(deserialized_v1)
        }
        ManifestVersion::V2 => Ok(toml::from_str(v1_or_v2_toml)?),
    }
}

/// Returns whether the given TOML content is a standalone component manifest
/// (`component.toml`) rather than an application manifest (`spin.toml`).
pub fn is_component_manifest(s: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct ComponentManifestProbe {
        #[allow(dead_code)]
        component_manifest_version: toml::Value,
    }

    toml::from_str::<ComponentManifestProbe>(s).is_ok()
}

/// Parses a standalone component manifest (`component.toml`) file into a
/// [`ComponentManifest`](schema::component::ComponentManifest).
pub fn component_manifest_from_file(
    path: impl AsRef<Path>,
) -> Result<schema::component::ComponentManifest, Error> {
    let manifest_str = std::fs::read_to_string(path)?;
    component_manifest_from_str(&manifest_str)
}

/// Parses a standalone component manifest (`component.toml`) into a
/// [`ComponentManifest`](schema::component::ComponentManifest).
pub fn component_manifest_from_str(
    toml_str: &str,
) -> Result<schema::component::ComponentManifest, Error> {
    Ok(toml::from_str(toml_str)?)
}

/// A Spin manifest schema version.
#[derive(Debug, PartialEq)]
pub enum ManifestVersion {
    /// Spin manifest schema version 1.
    V1,
    /// Spin manifest schema version 2.
    V2,
}

impl ManifestVersion {
    /// Detects the Spin manifest schema version of the given TOML content.
    pub fn detect(s: &str) -> Result<Self, Error> {
        let schema::VersionProbe {
            spin_manifest_version,
        } = toml::from_str(s)?;
        if spin_manifest_version.as_str() == Some("1") {
            Ok(Self::V1)
        } else if spin_manifest_version.as_integer() == Some(2) {
            Ok(Self::V2)
        } else {
            Err(Error::InvalidVersion(spin_manifest_version.to_string()))
        }
    }
}
