use std::fmt::Display;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use wasm_pkg_common::{package::PackageRef, registry::Registry};

/// Variable definition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    /// `required = true`
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// `default = "default value"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// `secret = true`
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

/// Component source
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged)]
pub enum ComponentSource {
    /// `"local.wasm"`
    Local(String),
    /// `{ ... }`
    Remote {
        /// `url = "https://example.test/remote.wasm"`
        url: String,
        /// `digest = `"sha256:abc123..."`
        digest: String,
    },
    /// `{ ... }`
    Registry {
        /// `registry = "example.com"`
        #[schemars(with = "Option<String>")]
        registry: Option<Registry>,
        /// `package = "example:component"`
        #[schemars(with = "String")]
        package: PackageRef,
        /// `version = "1.2.3"`
        version: String,
    },
}

impl Display for ComponentSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComponentSource::Local(path) => write!(f, "{path:?}"),
            ComponentSource::Remote { url, digest } => write!(f, "{url:?} with digest {digest:?}"),
            ComponentSource::Registry {
                registry,
                package,
                version,
            } => {
                let registry_suffix = match registry {
                    None => "default registry".to_owned(),
                    Some(r) => format!("registry {r:?}"),
                };
                write!(f, "\"{package}@{version}\" from {registry_suffix}")
            }
        }
    }
}

/// WASI files mount
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged)]
pub enum WasiFilesMount {
    /// `"images/*.png"`
    Pattern(String),
    /// `{ ... }`
    Placement {
        /// `source = "content/dir"`
        source: String,
        /// `destination = "/"`
        destination: String,
    },
}

/// Component build configuration
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentBuildConfig {
    /// `command = "cargo build"`
    pub command: Commands,
    /// `workdir = "components/main"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// watch = ["src/**/*.rs"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watch: Vec<String>,
}

impl ComponentBuildConfig {
    /// The commands to execute for the build
    pub fn commands(&self) -> impl ExactSizeIterator<Item = &String> {
        let as_vec = match &self.command {
            Commands::Single(cmd) => vec![cmd],
            Commands::Multiple(cmds) => cmds.iter().collect(),
        };
        as_vec.into_iter()
    }
}

/// Component build command or commands
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Commands {
    /// `command = "cargo build"`
    Single(String),
    /// `command = ["cargo build", "wac encode compose-deps.wac -d my:pkg=app.wasm --registry fermyon.com"]`
    Multiple(Vec<String>),
}

fn is_false(v: &bool) -> bool {
    !*v
}
