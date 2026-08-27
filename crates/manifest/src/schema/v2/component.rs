use anyhow::anyhow;
use itertools::Itertools;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ComponentBuildConfig, ComponentDependency, ComponentSource, DependencyName,
    DependencyPackageName, LowerSnakeId, Map, TargetEnvironmentRef, WasiFilesMount, json_schema,
    kebab_or_snake_case,
};

/// A Spin component.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// The file, package, or URL containing the component Wasm binary.
    ///
    /// Example: `source = "bin/cart.wasm"`
    ///
    /// Learn more: https://spinframework.dev/writing-apps#the-component-source
    pub source: ComponentSource,
    /// A human-readable description of the component.
    ///
    /// Example: `description = "Shopping cart"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Configuration variables available to the component. Names must be
    /// in `lower_snake_case`. Values are strings, and may refer
    /// to application variables using `{{ ... }}` syntax.
    ///
    /// `variables = { users_endpoint = "https://{{ api_host }}/users"}`
    ///
    /// Learn more: https://spinframework.dev/variables#adding-variables-to-your-applications
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<LowerSnakeId, String>,
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
    /// Deprecated. Use `allowed_outbound_hosts` instead.
    ///
    /// Example: `allowed_http_hosts = ["example.com"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[deprecated]
    pub allowed_http_hosts: Vec<String>,
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
    /// The AI models which the component is allowed to access. For local execution, you must
    /// download all models; for hosted execution, you should check which models are available
    /// in your target environment.
    ///
    /// Example: `ai_models = ["llama2-chat"]`
    ///
    /// Learn more: https://spinframework.dev/serverless-ai-api-guide#using-serverless-ai-from-applications
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<json_schema::AIModel>")]
    pub ai_models: Vec<String>,
    /// The Spin environments with which the component must be compatible.
    /// If present, this overrides the default application targets (they are not combined).
    ///
    /// Example: `targets = ["spin-up:3.3", "spinkube:0.4"]`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<TargetEnvironmentRef>>,
    /// The component build configuration.
    ///
    /// Learn more: https://spinframework.dev/build
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<ComponentBuildConfig>,
    /// Settings for custom tools or plugins. Spin ignores this field.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(schema_with = "json_schema::map_of_toml_tables")]
    pub tool: Map<String, toml::Table>,
    /// If true, dependencies can invoke Spin APIs with the same permissions as the main
    /// component. If false, dependencies have no permissions (e.g. network,
    /// key-value stores, SQLite databases).
    ///
    /// Learn more: https://spinframework.dev/writing-apps#dependency-permissions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies_inherit_configuration: Option<bool>,
    /// Specifies how to satisfy Wasm Component Model imports of this component.
    ///
    /// Learn more: https://spinframework.dev/writing-apps#using-component-dependencies
    #[serde(default, skip_serializing_if = "ComponentDependencies::is_empty")]
    pub dependencies: ComponentDependencies,
    /// Override values to use when building or running a named build profile.
    ///
    /// Example: `profile.debug.build.command = "npm run build-debug"`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(crate) profile: Map<String, ComponentProfileOverride>,
}

impl Component {
    /// Combine `allowed_outbound_hosts` with the deprecated `allowed_http_hosts` into
    /// one array all normalized to the syntax of `allowed_outbound_hosts`.
    pub fn normalized_allowed_outbound_hosts(&self) -> anyhow::Result<Vec<String>> {
        #[allow(deprecated)]
        let normalized =
            crate::compat::convert_allowed_http_to_allowed_hosts(&self.allowed_http_hosts, false)?;
        if !normalized.is_empty() {
            terminal::warn!(
                "Use of the deprecated field `allowed_http_hosts` - to fix, \
            replace `allowed_http_hosts` with `allowed_outbound_hosts = {normalized:?}`",
            )
        }

        Ok(self
            .allowed_outbound_hosts
            .iter()
            .cloned()
            .chain(normalized)
            .collect())
    }
}

/// Customisations for a Spin component in a non-default profile.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentProfileOverride {
    /// The file, package, or URL containing the component Wasm binary.
    ///
    /// Example: `source = "bin/debug/cart.wasm"`
    ///
    /// Learn more: https://spinframework.dev/writing-apps#the-component-source
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<ComponentSource>,

    /// Environment variables for the Wasm module to be overridden in this profile.
    /// Environment variables specified in the default profile will still be set
    /// if not overridden here.
    ///
    /// `environment = { DB_URL = "mysql://spin:spin@localhost/dev" }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(crate) environment: Map<String, String>,

    /// Wasm Component Model imports to be overridden in this profile.
    /// Dependencies specified in the default profile will still be composed
    /// if not overridden here.
    ///
    /// Learn more: https://spinframework.dev/writing-apps#using-component-dependencies
    #[serde(default, skip_serializing_if = "ComponentDependencies::is_empty")]
    pub(crate) dependencies: ComponentDependencies,

    /// The command or commands for building the component in non-default profiles.
    /// If a component has no special build instructions for a profile, the
    /// default build command is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) build: Option<ComponentProfileBuildOverride>,
}

/// Customisations for a Spin component build in a non-default profile.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentProfileBuildOverride {
    /// The command or commands to build the component in a named profile. If multiple commands
    /// are specified, they are run sequentially from left to right.
    ///
    /// Example: `build.command = "cargo build"`
    ///
    /// Learn more: https://spinframework.dev/build#setting-up-for-spin-build
    pub(crate) command: crate::schema::common::Commands,
}

/// Component dependencies
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ComponentDependencies {
    /// `dependencies = { "foo:bar" = ">= 0.1.0" }`
    pub inner: Map<DependencyName, ComponentDependency>,
}

impl ComponentDependencies {
    /// This method validates the correct specification of dependencies in a
    /// component section of the manifest. See the documentation on the methods
    /// called for more information on the specific checks.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        self.ensure_plain_names_have_package()?;
        self.ensure_package_names_no_export()?;
        self.ensure_disjoint()?;
        Ok(())
    }

    /// This method ensures that all dependency names in plain form (e.g.
    /// "foo-bar") do not map to a `ComponentDependency::Version`, or a
    /// `ComponentDependency::Package` where the `package` is `None`.
    fn ensure_plain_names_have_package(&self) -> anyhow::Result<()> {
        for (dependency_name, dependency) in self.inner.iter() {
            let DependencyName::Plain(plain) = dependency_name else {
                continue;
            };
            match dependency {
                ComponentDependency::Package { package, .. } if package.is_none() => {}
                ComponentDependency::Version(_) => {}
                _ => continue,
            }
            anyhow::bail!("dependency {plain:?} must specify a package name");
        }
        Ok(())
    }

    /// This method ensures that dependency names in the package form (e.g.
    /// "foo:bar" or "foo:bar@0.1.0") do not map to specific exported
    /// interfaces, e.g. `"foo:bar = { ..., export = "my-export" }"` is invalid.
    fn ensure_package_names_no_export(&self) -> anyhow::Result<()> {
        for (dependency_name, dependency) in self.inner.iter() {
            if let DependencyName::Package(name) = dependency_name
                && name.interface.is_none()
            {
                let export = match dependency {
                    ComponentDependency::Package { export, .. } => export,
                    ComponentDependency::Local { export, .. } => export,
                    _ => continue,
                };

                anyhow::ensure!(
                    export.is_none(),
                    "using an export to satisfy the package dependency {dependency_name:?} is not currently permitted",
                );
            }
        }
        Ok(())
    }

    /// This method ensures that dependencies names do not conflict with each other. That is to say
    /// that two dependencies of the same package must have disjoint versions or interfaces.
    fn ensure_disjoint(&self) -> anyhow::Result<()> {
        for [this, other] in self.inner.keys().array_combinations::<2>() {
            let (DependencyName::Package(this), DependencyName::Package(other)) = (this, other)
            else {
                continue;
            };

            if this.package == other.package {
                Self::check_disjoint(this, other)?;
            }
        }
        Ok(())
    }

    pub(crate) fn check_disjoint(
        this: &DependencyPackageName,
        other: &DependencyPackageName,
    ) -> anyhow::Result<()> {
        assert_eq!(this.package, other.package);

        if let (Some(this_ver), Some(other_ver)) = (this.version.clone(), other.version.clone())
            && Self::normalize_compatible_version(this_ver)
                != Self::normalize_compatible_version(other_ver)
        {
            return Ok(());
        }

        if let (Some(this_itf), Some(other_itf)) =
            (this.interface.as_ref(), other.interface.as_ref())
            && this_itf != other_itf
        {
            return Ok(());
        }

        Err(anyhow!("{this:?} dependency conflicts with {other:?}"))
    }

    /// Normalize version to perform a compatibility check against another version.
    ///
    /// See backwards comptabilitiy rules at https://semver.org/
    fn normalize_compatible_version(mut version: semver::Version) -> semver::Version {
        version.build = semver::BuildMetadata::EMPTY;

        if version.pre != semver::Prerelease::EMPTY {
            return version;
        }
        if version.major > 0 {
            version.minor = 0;
            version.patch = 0;
            return version;
        }

        if version.minor > 0 {
            version.patch = 0;
            return version;
        }

        version
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
