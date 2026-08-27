use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Map, TargetEnvironmentRef, json_schema};

/// App details
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppDetails {
    /// The name of the application.
    ///
    /// Example: `name = "my-app"`
    pub name: String,
    /// The application version. This should be a valid semver version.
    ///
    /// Example: `version = "1.0.0"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// A human-readable description of the application.
    ///
    /// Example: `description = "App description"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The author(s) of the application.
    ///
    /// `authors = ["author@example.com"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// The Spin environments with which application components must be compatible
    /// unless otherwise specified. Individual components may express different
    /// requirements: these override the application-level default.
    ///
    /// Example: `targets = ["spin-up:3.3", "spinkube:0.4"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<TargetEnvironmentRef>,
    /// Application-level settings for the trigger types used in the application.
    /// The possible values are trigger type-specific.
    ///
    /// Example:
    ///
    /// ```ignore
    /// [application.triggers.redis]
    /// address = "redis://notifications.example.com:6379"
    /// ```
    ///
    /// Learn more (Redis example): https://spinframework.dev/redis-trigger#setting-a-default-server
    #[serde(rename = "trigger", default, skip_serializing_if = "Map::is_empty")]
    #[schemars(schema_with = "json_schema::map_of_toml_tables")]
    pub trigger_global_configs: Map<String, toml::Table>,
    /// Settings for custom tools or plugins. Spin ignores this field.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(schema_with = "json_schema::map_of_toml_tables")]
    pub tool: Map<String, toml::Table>,
}
