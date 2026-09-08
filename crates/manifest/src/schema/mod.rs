//! Serialization types for the Spin manifest file formats: the application
//! manifest (`spin.toml`) and the standalone component manifest (`component.toml`).

use serde::Deserialize;

/// Serialization types for a standalone component manifest (`component.toml`).
pub mod component;
/// Serialization types for the Spin manifest V1.
pub mod v1;
/// Serialization types for the Spin manifest V2.
pub mod v2;

// Types common between manifest versions. Re-exported from versioned modules
// to make them easier to split if necessary.
pub(crate) mod common;
mod json_schema;

// Serde serialise-deserialise modules
mod kebab_or_snake_case;
mod one_or_many;

#[derive(Deserialize)]
pub(crate) struct VersionProbe {
    #[serde(alias = "spin_version")]
    pub spin_manifest_version: toml::Value,
}
