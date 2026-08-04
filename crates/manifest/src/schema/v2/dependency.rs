use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use spin_serde::{KebabId, LowerSnakeId};

use super::{Map, WasiFilesMount, json_schema, kebab_or_snake_case};

/// Specifies how to satisfy an import dependency of the component. This may be one of:
///
/// - A semantic versioning constraint for the package version to use. Spin fetches the latest matching version of the package whose name matches the dependency name from the default registry.
///
/// Example: `"my:dep/import" = ">= 0.1.0"`
///
/// - A package from a registry.
///
/// Example: `"my:dep/import" = { version = "0.1.0", registry = "registry.io", ...}`
///
/// - A package from a filesystem path.
///
/// Example: `"my:dependency" = { path = "path/to/component.wasm", export = "my-export" }`
///
/// - A component in the application. The referenced component binary is composed: additional
///   configuration such as files, networking, storage, etc. are ignored. This is intended
///   primarily as a convenience for including dependencies in the manifest so that they
///   can be built using `spin build`.
///
/// Example: `"my:dependency" = { component = "my-dependency", export = "my-export" }`
///
/// - A package from an HTTP URL.
///
/// Example: `"my:import" = { url = "https://example.com/component.wasm", sha256 = "sha256:..." }`
///
/// Learn more: https://spinframework.dev/v3/writing-apps#using-component-dependencies
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ComponentDependency {
    /// `... = ">= 0.1.0"`
    #[schemars(description = "")] // schema docs are on the parent
    Version(String),
    /// `... = { version = "0.1.0", registry = "registry.io", ...}`
    #[schemars(description = "")] // schema docs are on the parent
    Package {
        /// A semantic versioning constraint for the package version to use. Required. Spin
        /// fetches the latest matching version from the specified registry, or from
        /// the default registry if no registry is specified.
        ///
        /// Example: `"my:dep/import" = { version = ">= 0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        version: String,
        /// The registry that hosts the package. If omitted, this defaults to your
        /// system default registry.
        ///
        /// Example: `"my:dep/import" = { registry = "registry.io", version = "0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        registry: Option<String>,
        /// The name of the package to use. If omitted, this defaults to the package name of the
        /// imported interface.
        ///
        /// Example: `"my:dep/import" = { package = "your:implementation", version = "0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        package: Option<String>,
        /// The name of the export in the package. If omitted, this defaults to the name of the import.
        ///
        /// Example: `"my:dep/import" = { export = "your:impl/export", version = "0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        export: Option<String>,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
    /// `... = { path = "path/to/component.wasm", export = "my-export" }`
    #[schemars(description = "")] // schema docs are on the parent
    Local {
        /// The path to the Wasm file that implements the dependency.
        ///
        /// Example: `"my:dep/import" = { path = "path/to/component.wasm" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-local-component
        path: PathBuf,
        /// The name of the export in the package. If omitted, this defaults to the name of the import.
        ///
        /// Example: `"my:dep/import" = { export = "your:impl/export", path = "path/to/component.wasm" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-local-component
        export: Option<String>,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
    /// `... = { url = "https://example.com/component.wasm", sha256 = "..." }`
    #[schemars(description = "")] // schema docs are on the parent
    HTTP {
        /// The URL to the Wasm component that implements the dependency.
        ///
        /// Example: `"my:dep/import" = { url = "https://example.com/component.wasm", sha256 = "sha256:..." }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-url
        url: String,
        /// The SHA256 digest of the Wasm file. This is required for integrity checking. Must begin with `sha256:`.
        ///
        /// Example: `"my:dep/import" = { sha256 = "sha256:...", ... }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-url
        digest: String,
        /// The name of the export in the package. If omitted, this defaults to the name of the import.
        ///
        /// Example: `"my:dep/import" = { export = "your:impl/export", ... }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-url
        export: Option<String>,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
    /// `... = { component = "my-dependency" }`
    #[schemars(description = "")] // schema docs are on the parent
    AppComponent {
        /// The ID of the component which implements the dependency.
        ///
        /// Example: `"my:dep/import" = { component = "my-dependency" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#using-component-dependencies
        component: KebabId,
        /// The name of the export in the package. If omitted, this defaults to the name of the import.
        ///
        /// Example: `"my:dep/import" = { export = "your:impl/export", component = "my-dependency" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#using-component-dependencies
        export: Option<String>,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
}

impl ComponentDependency {
    /// Returns the `inherit_configuration` field if present on this dependency variant.
    pub fn inherit_configuration(&self) -> Option<&InheritConfiguration> {
        match self {
            ComponentDependency::Version(_) => None,
            ComponentDependency::Package {
                inherit_configuration,
                ..
            }
            | ComponentDependency::Local {
                inherit_configuration,
                ..
            }
            | ComponentDependency::HTTP {
                inherit_configuration,
                ..
            }
            | ComponentDependency::AppComponent {
                inherit_configuration,
                ..
            } => inherit_configuration.as_ref(),
        }
    }

    /// TODO:
    pub fn capabilities(&self) -> Option<&DependencyCapabilities> {
        match self {
            ComponentDependency::Version(_) => None,
            ComponentDependency::Package { capabilities, .. }
            | ComponentDependency::Local { capabilities, .. }
            | ComponentDependency::HTTP { capabilities, .. }
            | ComponentDependency::AppComponent { capabilities, .. } => capabilities.as_ref(),
        }
    }

    /// Sets the `inherit_configuration` field on this dependency variant.
    /// No-op for `Version` variants.
    pub fn set_inherit_configuration(&mut self, value: InheritConfiguration) {
        match self {
            ComponentDependency::Version(_) => {}
            ComponentDependency::Package {
                inherit_configuration,
                ..
            }
            | ComponentDependency::Local {
                inherit_configuration,
                ..
            }
            | ComponentDependency::HTTP {
                inherit_configuration,
                ..
            }
            | ComponentDependency::AppComponent {
                inherit_configuration,
                ..
            } => {
                *inherit_configuration = Some(value);
            }
        }
    }
}

/// Specifies how to satisfy an import dependency of the component. This may be one of:
///
/// - A semantic versioning constraint for the package version to use. Spin fetches the latest matching version of the package whose name matches the dependency name from the default registry.
///
/// Example: `"my:dep/import" = ">= 0.1.0"`
///
/// - A package from a registry.
///
/// Example: `"my:dep/import" = { version = "0.1.0", registry = "registry.io", ...}`
///
/// - A package from a filesystem path.
///
/// Example: `"my:dependency" = { path = "path/to/component.wasm", export = "my-export" }`
///
/// - A component in the application. The referenced component binary is composed: additional
///   configuration such as files, networking, storage, etc. are ignored. This is intended
///   primarily as a convenience for including dependencies in the manifest so that they
///   can be built using `spin build`.
///
/// Example: `"my:dependency" = { component = "my-dependency", export = "my-export" }`
///
/// - A package from an HTTP URL.
///
/// Example: `"my:import" = { url = "https://example.com/component.wasm", sha256 = "sha256:..." }`
///
/// Learn more: https://spinframework.dev/v3/writing-apps#using-component-dependencies
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum TriggerDependency {
    /// `... = { version = "0.1.0", registry = "registry.io", ...}`
    #[schemars(description = "")] // schema docs are on the parent
    Package {
        /// A semantic versioning constraint for the package version to use. Required. Spin
        /// fetches the latest matching version from the specified registry, or from
        /// the default registry if no registry is specified.
        ///
        /// Example: `"my:dep/import" = { version = ">= 0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        version: String,
        /// The registry that hosts the package. If omitted, this defaults to your
        /// system default registry.
        ///
        /// Example: `"my:dep/import" = { registry = "registry.io", version = "0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        registry: Option<String>,
        /// The name of the package to use. If omitted, this defaults to the package name of the
        /// imported interface.
        ///
        /// Example: `"my:dep/import" = { package = "your:implementation", version = "0.1.0" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-registry
        package: String,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
    /// `... = { path = "path/to/component.wasm", export = "my-export" }`
    #[schemars(description = "")] // schema docs are on the parent
    Local {
        /// The path to the Wasm file that implements the dependency.
        ///
        /// Example: `"my:dep/import" = { path = "path/to/component.wasm" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-local-component
        path: PathBuf,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
    /// `... = { url = "https://example.com/component.wasm", sha256 = "..." }`
    #[schemars(description = "")] // schema docs are on the parent
    HTTP {
        /// The URL to the Wasm component that implements the dependency.
        ///
        /// Example: `"my:dep/import" = { url = "https://example.com/component.wasm", sha256 = "sha256:..." }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-url
        url: String,
        /// The SHA256 digest of the Wasm file. This is required for integrity checking. Must begin with `sha256:`.
        ///
        /// Example: `"my:dep/import" = { sha256 = "sha256:...", ... }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#dependencies-from-a-url
        digest: String,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
    /// `... = { component = "my-dependency" }`
    #[schemars(description = "")] // schema docs are on the parent
    AppComponent {
        /// The ID of the component which implements the dependency.
        ///
        /// Example: `"my:dep/import" = { component = "my-dependency" }`
        ///
        /// Learn more: https://spinframework.dev/writing-apps#using-component-dependencies
        component: KebabId,
        /// The set of configurations to inherit from the parent component. If omitted or set to `false`,
        /// no configurations will be inherited. If `true`, all configurations will be inherited.
        /// Selective inheritance can be specified by enumerating the configuration keys the dependency
        /// would like to inherit.
        ///
        /// Examples:
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = true }`
        ///     `"my:dep/import" = { version = "0.1.0", inherit_configuration = ["ai_models", "allowed_outbound_hosts"] }`
        inherit_configuration: Option<InheritConfiguration>,
        /// TODO:
        capabilities: Option<DependencyCapabilities>,
    },
}

/// The set of configurations to inherit from the parent component.
///
/// Can be specified as:
/// - `true` — inherit all configurations
/// - `false` — inherit no configurations (equivalent to omitting the field)
/// - `["key1", "key2"]` — inherit only the specified configuration keys
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InheritConfiguration {
    /// All or no configurations will be inherited, specified as `true` or `false`.
    All(bool),
    /// Only the specified configuration keys will be inherited from the parent component.
    Some(Vec<String>),
}

/// TODO: unify with component capabilities
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DependencyCapabilities {
    // TODO: -------------------------------------------------------------------------------
    /// Environment variables to be set for the Wasm module.
    ///
    /// `environment = { DB_URL = "mysql://spin:spin@localhost/dev" }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub environment: Map<String, String>,
    /// The files the component is allowed to read. Each list entry is either:
    ///
    /// - a glob pattern (e.g. "assets/**/*.jpg"); or
    ///
    /// - a source-destination pair indicating where a host directory should be mapped in the guest (e.g. { source = "assets", destination = "/" })
    ///
    /// Learn more: https://spinframework.dev/writing-apps#including-files-with-components
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<WasiFilesMount>,
    /// Any files or glob patterns that should not be available to the
    /// Wasm module at runtime, even though they match a `files`` entry.
    ///
    /// Example: `exclude_files = ["secrets/*"]`
    ///
    /// Learn more: https://spinframework.dev/writing-apps#including-files-with-components
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_files: Vec<String>,

    // END TODO: -------------------------------------------------------------------------------
    /// Configuration variables available to the component. Names must be
    /// in `lower_snake_case`. Values are strings, and may refer
    /// to application variables using `{{ ... }}` syntax.
    ///
    /// `variables = { users_endpoint = "https://{{ api_host }}/users"}`
    ///
    /// Learn more: https://spinframework.dev/variables#adding-variables-to-your-applications
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<LowerSnakeId, String>,

    /// The network destinations which the component is allowed to access.
    /// Each entry is in the form "(scheme)://(host)[:port]". Each element
    /// allows * as a wildcard e.g. "https://\*" (HTTPS on the default port
    /// to any destination) or "\*://localhost:\*" (any protocol to any port on
    /// localhost). The host part allows segment wildcards for subdomains
    /// e.g. "https://\*.example.com". Application variables are allowed using
    /// `{{ my_var }}`` syntax.
    ///
    /// Example: `allowed_outbound_hosts = ["redis://myredishost.com:6379"]`
    ///
    /// Learn more: https://spinframework.dev/http-outbound#granting-http-permissions-to-components
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<json_schema::AllowedOutboundHost>")]
    pub allowed_outbound_hosts: Vec<String>,

    /// The key-value stores which the component is allowed to access. Stores are identified
    /// by label e.g. "default" or "customer". Stores other than "default" must be mapped
    /// to a backing store in the runtime config.
    ///
    /// Example: `key_value_stores = ["default", "my-store"]`
    ///
    /// Learn more: https://spinframework.dev/kv-store-api-guide#custom-key-value-stores
    #[serde(
        default,
        with = "kebab_or_snake_case",
        skip_serializing_if = "Vec::is_empty"
    )]
    #[schemars(with = "Vec<json_schema::KeyValueStore>")]
    pub key_value_stores: Vec<String>,

    /// The SQLite databases which the component is allowed to access. Databases are identified
    /// by label e.g. "default" or "analytics". Databases other than "default" must be mapped
    /// to a backing store in the runtime config. Use "spin up --sqlite" to run database setup scripts.
    ///
    /// Example: `sqlite_databases = ["default", "my-database"]`
    ///
    /// Learn more: https://spinframework.dev/sqlite-api-guide#preparing-an-sqlite-database
    #[serde(
        default,
        with = "kebab_or_snake_case",
        skip_serializing_if = "Vec::is_empty"
    )]
    #[schemars(with = "Vec<json_schema::SqliteDatabase>")]
    pub sqlite_databases: Vec<String>,
}
