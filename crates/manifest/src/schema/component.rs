//! Serialization types for a standalone component manifest (`component.toml`).
//!
//! Unlike an application manifest (`spin.toml`), which describes a whole
//! application and all of its components, a component manifest describes a
//! single component in isolation: its metadata, how to build it, and what
//! capabilities it requires from a host application.
//!
//! All types in this module derive [`serde::Serialize`] / [`serde::Deserialize`]
//! (so a manifest can be round-tripped through TOML or JSON) and
//! [`schemars::JsonSchema`] (so a JSON Schema can be generated for tooling).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use spin_serde::FixedVersion;

use super::common::{Commands, ComponentBuildConfig, ComponentSource, WasiFilesMount};

/// A standalone component manifest (`component.toml`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifest {
    /// `component_manifest_version = 1`
    #[schemars(with = "usize", range(min = 1, max = 1))]
    pub component_manifest_version: FixedVersion<1>,
    /// `[component]`
    pub component: ComponentManifestDetails,
    /// `[build]`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<ComponentManifestBuild>,
    /// `[requires]`
    #[serde(default, skip_serializing_if = "ComponentRequires::is_empty")]
    pub requires: ComponentRequires,
}

/// Component details (`[component]`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifestDetails {
    /// The name of the component. This is used as the component's identifier and
    /// as the package name when publishing.
    ///
    /// Example: `name = "my-component"`
    pub name: String,
    /// The path to the Wasm file that is the component's artifact. Always
    /// required, as it is the file used when packaging the component.
    ///
    /// Example: `source = "target/wasm32-wasip2/release/component.wasm"`
    pub source: String,
    /// The component version. This should be a valid semver version.
    ///
    /// Example: `version = "1.0.0"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// A human-readable description of the component.
    ///
    /// Example: `description = "Component description"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The author(s) of the component.
    ///
    /// Example: `authors = ["author@example.com"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// The URL of the component's source repository.
    ///
    /// Example: `repository = "https://example.com/my-component"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repository: String,
    /// The license under which the component is distributed.
    ///
    /// Example: `license = "Apache-2.0"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

impl ComponentManifestDetails {
    /// The component source. A component manifest always specifies a source, as
    /// it is the artifact used when packaging the component.
    pub fn source(&self) -> ComponentSource {
        ComponentSource::Local(self.source.clone())
    }
}

/// Component build configuration (`[build]`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentManifestBuild {
    /// The command or commands to build the component. If multiple commands
    /// are specified, they are run sequentially from left to right.
    ///
    /// Example: `command = "cargo build --release"`
    pub command: Commands,
    /// The working directory for the build command. If omitted, the build working
    /// directory is the directory containing `component.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
}

impl ComponentManifestBuild {
    /// The build configuration expressed as an application-style
    /// [`ComponentBuildConfig`].
    pub fn to_build_config(&self) -> ComponentBuildConfig {
        ComponentBuildConfig {
            command: self.command.clone(),
            workdir: self.workdir.clone(),
            watch: Vec::new(),
        }
    }
}

/// The capabilities a component requires from a host application (`[requires]`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentRequires {
    /// Configuration variables the component consumes.
    ///
    /// Example: `variables = ["api_key", { name = "region", default = "us", secret = false }]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<RequiredVariable>,
    /// The names of key-value stores the component needs access to.
    ///
    /// Example: `key_value_stores = ["default"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_value_stores: Vec<String>,
    /// The names of SQLite databases the component needs access to.
    ///
    /// Example: `sql_variables = ["default"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sql_variables: Vec<String>,
    /// The environments the component needs.
    ///
    /// Example: `environments = ["staging", { name = "region", default = "us" }]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environments: Vec<RequiredEnvironment>,
    /// The names of AI models the component needs access to.
    ///
    /// Example: `ai_models = ["llama2-chat"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_models: Vec<String>,
    /// The hosts the component is allowed to make outbound network requests to.
    ///
    /// Example: `allowed_outbound_hosts = ["https://example.com:443"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_outbound_hosts: Vec<String>,
    /// The files the component is allowed to read. Each entry is either a glob
    /// pattern or a source-to-destination directory mapping.
    ///
    /// Example: `files = ["assets/**/*", { source = "local/path", destination = "/mounted/path" }]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<WasiFilesMount>,
}

impl ComponentRequires {
    /// Whether the component declares no requirements.
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
            && self.key_value_stores.is_empty()
            && self.sql_variables.is_empty()
            && self.environments.is_empty()
            && self.ai_models.is_empty()
            && self.allowed_outbound_hosts.is_empty()
            && self.files.is_empty()
    }
}

/// A configuration variable required by a component. This is either the bare
/// name of the variable, or a table giving the name alongside a default value
/// and/or secrecy flag.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RequiredVariable {
    /// `"my_variable"`
    #[schemars(description = "")] // schema docs are on the parent
    Name(String),
    /// `{ name = "my_variable", default = "value", secret = true }`
    #[schemars(description = "")] // schema docs are on the parent
    Detailed(RequiredVariableDetails),
}

/// The detailed form of a [`RequiredVariable`].
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredVariableDetails {
    /// The name of the variable.
    pub name: String,
    /// The value used if none is supplied at runtime. If omitted, a value must
    /// be provided at runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Whether the variable should be treated as sensitive.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

/// An environment required by a component. This is either the bare name of the
/// environment, or a table giving the name alongside a default value.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RequiredEnvironment {
    /// `"my_environment"`
    #[schemars(description = "")] // schema docs are on the parent
    Name(String),
    /// `{ name = "my_environment", default = "value" }`
    #[schemars(description = "")] // schema docs are on the parent
    Detailed(RequiredEnvironmentDetails),
}

/// The detailed form of a [`RequiredEnvironment`].
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequiredEnvironmentDetails {
    /// The name of the environment.
    pub name: String,
    /// The value used if none is supplied at runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
component_manifest_version = 1

[component]
name = "github-oauth-middleware"
source = "something/something.wasm"
version = "0.1.0"
authors = ["Michelle Dhanani <mdhanani@akamai.com>"]
description = "github-oauth-middleware"
repository = "https://github.com/michellen/github-oauth-middleware"
license = "apache2"

[build]
command = "cargo build --target wasm32-wasip2 --release"

[requires]
variables = [
    "name of instance of capability",
    { name = "variable_2", default = "foo,bar", secret = true },
]
key_value_stores = ["store1", "store2"]
sql_variables = ["sql1", "sql2"]
environments = [{ name = "env1", default = "something" }, "env2"]
ai_models = ["some-ai"]
allowed_outbound_hosts = ["https://example.com:443", "redis://redis.example.com:6379"]
files = ["assets/**/*", { source = "local/path", destination = "/mounted/path" }]
"#;

    #[test]
    fn parses_example_component_manifest() {
        let manifest: ComponentManifest = toml::from_str(EXAMPLE).unwrap();

        assert_eq!(manifest.component.name, "github-oauth-middleware");
        let requires = &manifest.requires;
        assert_eq!(requires.variables.len(), 2);
        assert_eq!(requires.key_value_stores, ["store1", "store2"]);
        assert_eq!(requires.sql_variables, ["sql1", "sql2"]);
        assert_eq!(requires.environments.len(), 2);
        assert_eq!(requires.ai_models, ["some-ai"]);
        assert_eq!(requires.allowed_outbound_hosts.len(), 2);
        assert_eq!(requires.files.len(), 2);
    }

    #[test]
    fn serializes_to_json() {
        let manifest: ComponentManifest = toml::from_str(EXAMPLE).unwrap();
        let json = serde_json::to_string_pretty(&manifest).unwrap();

        // Round-trips back through JSON to an equivalent model.
        let from_json: ComponentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(from_json.component.name, manifest.component.name);
        assert_eq!(from_json.requires.variables.len(), 2);
    }
}
