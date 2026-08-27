use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use spin_serde::KebabId;

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
