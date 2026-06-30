use std::collections::{HashMap, HashSet};

use anyhow::Context;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(tag = "manifest_version")]
pub(crate) enum RawTemplateManifest {
    /// A manifest with API version 1.
    #[serde(rename = "1")]
    V1(RawTemplateManifestV1),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) struct RawTemplateManifestV1 {
    pub id: String,
    pub description: Option<String>,
    pub trigger_type: Option<String>,
    pub tags: Option<HashSet<String>>,
    pub new_application: Option<RawTemplateVariant>,
    pub add_component: Option<RawTemplateVariant>,
    pub parameters: Option<IndexMap<String, RawParameter>>,
    pub custom_filters: Option<serde::de::IgnoredAny>, // kept for error messaging
    pub outputs: Option<IndexMap<String, RawExtraOutput>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) struct RawTemplateVariant {
    pub supported: Option<bool>,
    pub copy_into: Option<RawCopyInto>,
    pub skip_files: Option<Vec<String>>,
    pub skip_parameters: Option<Vec<String>>,
    pub snippets: Option<HashMap<String, String>>,
    pub conditions: Option<HashMap<String, RawConditional>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum RawCopyInto {
    Root,
    Own,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) struct RawConditional {
    pub condition: RawCondition,
    pub skip_files: Option<Vec<String>>,
    pub skip_parameters: Option<Vec<String>>,
    pub skip_snippets: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "snake_case",
    try_from = "toml::Value"
)]
pub(crate) enum RawCondition {
    ManifestEntryExists(String),
}

impl TryFrom<toml::Value> for RawCondition {
    type Error = anyhow::Error;

    fn try_from(value: toml::Value) -> Result<Self, Self::Error> {
        let Some(table) = value.as_table() else {
            anyhow::bail!("Invalid condition: should be a single-entry table");
        };
        if table.keys().len() != 1 {
            anyhow::bail!("Invalid condition: should be a single-entry table");
        }
        let Some(value) = table.get("manifest_entry_exists") else {
            anyhow::bail!("Invalid condition: unknown condition type");
        };
        let Some(path) = value.as_str() else {
            anyhow::bail!(
                "Invalid condition: 'manifest_entry_exists' should be a dotted-path string"
            );
        };
        Ok(Self::ManifestEntryExists(path.to_owned()))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) struct RawParameter {
    #[serde(rename = "type")]
    pub data_type: String,
    pub prompt: String,
    #[serde(rename = "default")]
    pub default_value: Option<String>,
    pub pattern: Option<String>,
    pub allowed_values: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "action")]
pub(crate) enum RawExtraOutput {
    CreateDir(RawCreateDir),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) struct RawCreateDir {
    pub path: String,
    pub at: Option<CreateLocation>,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum CreateLocation {
    #[default]
    Component,
    Manifest,
}

pub(crate) fn parse_manifest_toml(text: impl AsRef<str>) -> anyhow::Result<RawTemplateManifest> {
    toml::from_str(text.as_ref()).context("Failed to parse template manifest TOML")
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", untagged)]
pub(crate) enum RawInstalledFrom {
    Git { git: String },
    File { dir: String },
    RemoteTar { url: String },
}

pub(crate) fn parse_installed_from(text: impl AsRef<str>) -> Option<RawInstalledFrom> {
    // If we can't load it then it's not worth an error
    toml::from_str(text.as_ref()).ok()
}
