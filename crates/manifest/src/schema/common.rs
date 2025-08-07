use std::fmt::Display;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use wasm_pkg_common::{package::PackageRef, registry::Registry};

use super::json_schema;

/// The name of the application variable.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    /// A brief description of the variable.
    ///
    /// Example: `description = "This is a variable"`
    ///
    /// Learn more: https://spinframework.dev/variables#adding-variables-to-your-applications
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether a value must be supplied at runtime. If not specified, required defaults
    /// to `false`, and `default` must be provided.
    ///
    /// Example: `required = true`
    ///
    /// Learn more: https://spinframework.dev/variables#adding-variables-to-your-applications
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// The value of the variable if no value is supplied at runtime. If specified,
    /// the value must be a string. If not specified, `required`` must be `true`.
    ///
    /// Example: `default = "default value"`
    ///
    /// Learn more: https://spinframework.dev/variables#adding-variables-to-your-applications
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// If set, this variable should be treated as sensitive.
    ///
    /// Example: `secret = true`
    ///
    /// Learn more: https://spinframework.dev/variables#adding-variables-to-your-applications
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

/// The file, package, or URL containing the component Wasm binary. This may be:
///
/// - The path to a Wasm file (relative to the manifest file)
///
/// Example: `source = "bin/cart.wasm"`
///
/// - The URL of a Wasm file downloadable over HTTP, accompanied by a digest to ensure integrity
///
/// Example: `source = { url = "https://example.com/example.wasm", digest = "sha256:6503...2375" }`
///
/// - The registry, package and version of a component from a registry
///
/// Example: `source = { registry = "ttl.sh", package = "user:registrytest", version="1.0.0" }`
///
/// Learn more: https://spinframework.dev/writing-apps#the-component-source
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged)]
pub enum ComponentSource {
    /// `source = "bin/cart.wasm"`
    #[schemars(description = "")] // schema docs are on the parent
    Local(String),
    /// `source = { url = "https://example.com/example.wasm", digest = "sha256:6503...2375" }`
    #[schemars(description = "")] // schema docs are on the parent
    Remote {
        /// The URL of the Wasm component binary.
        ///
        /// Example: `url = "https://example.test/remote.wasm"`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#the-component-source
        url: String,
        /// The SHA256 digest of the Wasm component binary. This must be prefixed with `sha256:`.
        ///
        /// Example: `digest = `"sha256:abc123..."`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#the-component-source
        digest: String,
    },
    /// `source = { registry = "ttl.sh", package = "user:registrytest", version="1.0.0" }`
    #[schemars(description = "")] // schema docs are on the parent
    Registry {
        /// The registry containing the Wasm component binary.
        ///
        /// Example: `registry = "example.com"`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#the-component-source
        #[schemars(with = "Option<String>")]
        registry: Option<Registry>,
        /// The package containing the Wasm component binary.
        ///
        /// Example: `package = "example:component"`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#the-component-source
        #[schemars(with = "String")]
        package: PackageRef,
        /// The version of the package containing the Wasm component binary.
        ///
        /// Example: `version = "1.2.3"`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#the-component-source
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

/// The files the component is allowed to read. Each list entry is either:
///
/// - a glob pattern (e.g. "assets/**/*.jpg"); or
///
/// - a source-destination pair indicating where a host directory should be mapped in the guest (e.g. { source = "assets", destination = "/" })
///
/// Learn more: https://spinframework.dev/writing-apps#including-files-with-components
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged)]
pub enum WasiFilesMount {
    /// `"images/*.png"`
    #[schemars(description = "")] // schema docs are on the parent
    Pattern(String),
    /// `{ ... }`
    #[schemars(description = "")] // schema docs are on the parent
    Placement {
        /// The directory to be made available in the guest.
        ///
        /// Example: `source = "content/dir"`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#including-files-with-components
        source: String,
        /// The path where the `source` directory appears in the guest. Must be absolute.
        ///
        /// `destination = "/"`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#including-files-with-components
        destination: String,
    },
}

/// Component build configuration
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentBuildConfig {
    /// The command or commands to build the application. If multiple commands
    /// are specified, they are run sequentially from left to right.
    ///
    /// Example: `command = "cargo build"`, `command = ["npm install", "npm run build"]`
    ///
    /// Learn more: https://spinframework.dev/build#setting-up-for-spin-build
    pub command: Commands,
    /// The working directory for the build command. If omitted, the build working
    /// directory is the directory containing `spin.toml`.
    ///
    /// Example: `workdir = "components/main"
    ///
    /// Learn more: https://spinframework.dev/build#overriding-the-working-directory
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// Source files to use in `spin watch`. This is a set of paths or glob patterns (relative
    /// to the build working directory). A change to any matching file causes
    /// `spin watch` to rebuild the application before restarting the application.
    ///
    /// Example: `watch = ["src/**/*.rs"]`
    ///
    /// Learn more: https://spinframework.dev/running-apps#monitoring-applications-for-changes
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<json_schema::WatchCommand>")]
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

/// The command or commands to build the application. If multiple commands
/// are specified, they are run sequentially from left to right.
///
/// Example: `command = "cargo build"`, `command = ["npm install", "npm run build"]`
///
/// Learn more: https://spinframework.dev/build#setting-up-for-spin-build
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Commands {
    /// `command = "cargo build"`
    #[schemars(description = "")] // schema docs are on the parent
    Single(String),
    /// `command = ["cargo build", "wac encode compose-deps.wac -d my:pkg=app.wasm --registry fermyon.com"]`
    #[schemars(description = "")] // schema docs are on the parent
    Multiple(Vec<String>),
}

fn is_false(v: &bool) -> bool {
    !*v
}
