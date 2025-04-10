use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use spin_serde::{DependencyName, DependencyPackageName, FixedVersion, LowerSnakeId};
pub use spin_serde::{KebabId, SnakeId};
use std::path::PathBuf;

pub use super::common::{ComponentBuildConfig, ComponentSource, Variable, WasiFilesMount};

pub(crate) type Map<K, V> = indexmap::IndexMap<K, V>;

/// App manifest
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    /// `spin_manifest_version = 2`
    #[schemars(with = "usize", range = (min = 2, max = 2))]
    pub spin_manifest_version: FixedVersion<2>,
    /// `[application]`
    pub application: AppDetails,
    /// `[variables]`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<LowerSnakeId, Variable>,
    /// `[[trigger.<type>]]`
    #[serde(rename = "trigger")]
    #[schemars(with = "schema::TriggerSchema")]
    pub triggers: Map<String, Vec<Trigger>>,
    /// `[component.<id>]`
    #[serde(rename = "component")]
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub components: Map<KebabId, Component>,
}

impl AppManifest {
    /// This method ensures that the dependencies of each component are valid.
    pub fn validate_dependencies(&self) -> anyhow::Result<()> {
        for (component_id, component) in &self.components {
            component
                .dependencies
                .validate()
                .with_context(|| format!("component {component_id:?} has invalid dependencies"))?;
        }
        Ok(())
    }
}

/// App details
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppDetails {
    /// `name = "my-app"`
    pub name: String,
    /// `version = "1.0.0"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// `description = "App description"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// `authors = ["author@example.com"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// `[application.triggers.<type>]`
    #[serde(rename = "trigger", default, skip_serializing_if = "Map::is_empty")]
    #[schemars(schema_with = "map_of_toml_tables_schema")]
    pub trigger_global_configs: Map<String, toml::Table>,
    /// Settings for custom tools or plugins. Spin ignores this field.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(schema_with = "map_of_toml_tables_schema")]
    pub tool: Map<String, toml::Table>,
}

/// Trigger configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    /// `id = "trigger-id"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// `component = ...`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentSpec>,
    /// `components = { ... }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub components: Map<String, OneOrManyComponentSpecs>,
    /// Opaque trigger-type-specific config
    #[serde(flatten)]
    pub config: toml::Table,
}

/// One or many `ComponentSpec`(s)
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct OneOrManyComponentSpecs(
    #[serde(with = "one_or_many")]
    #[schemars(schema_with = "one_or_many_schema::<ComponentSpec>")]
    pub Vec<ComponentSpec>,
);

fn one_or_many_schema<T: schemars::JsonSchema>(
    gen: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
            one_of: Some(vec![
                gen.subschema_for::<T>(),
                gen.subschema_for::<Vec<T>>(),
            ]),
            ..Default::default()
        })),
        ..Default::default()
    })
}

fn toml_table_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::Object,
        ))),
        ..Default::default()
    })
}

fn map_of_toml_tables_schema(
    _gen: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::Object,
        ))),
        ..Default::default()
    })
}

/// TODO: there may be a way to do this without dead code using `schemars::extend` or
/// something like that?
#[allow(dead_code)]
mod schema {
    use super::{toml_table_schema, ComponentSpec, Map, OneOrManyComponentSpecs};
    use schemars::JsonSchema;

    #[derive(JsonSchema)]
    pub struct TriggerSchema {
        /// HTTP triggers
        #[schemars(default)]
        http: Vec<HttpTriggerSchema>,
        /// Redis triggers
        #[schemars(default)]
        redis: Vec<RedisTriggerSchema>,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    struct HttpTriggerSchema {
        /// `id = "trigger-id"`
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub id: String,
        /// `component = ...`
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub component: Option<ComponentSpec>,
        /// `components = { ... }`
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        pub components: Map<String, OneOrManyComponentSpecs>,
        /// `route = "/user/:name/..."`
        route: HttpRouteSchema,
        /// `executor = { type = "wagi" }
        #[schemars(default, schema_with = "toml_table_schema")]
        executor: Option<toml::Table>,
    }

    #[derive(JsonSchema)]
    #[schemars(untagged)]
    enum HttpRouteSchema {
        /// `route = "/user/:name/..."`
        Route(String),
        /// `route = { private = true }`
        Private(HttpPrivateEndpoint),
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    struct HttpPrivateEndpoint {
        /// Whether the private endpoint is private. This must be true.
        pub private: bool,
    }

    #[derive(JsonSchema)]
    #[schemars(deny_unknown_fields)]
    struct RedisTriggerSchema {
        /// `id = "trigger-id"`
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub id: String,
        /// `component = ...`
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub component: Option<ComponentSpec>,
        /// `components = { ... }`
        #[serde(default, skip_serializing_if = "Map::is_empty")]
        pub components: Map<String, OneOrManyComponentSpecs>,
        /// `channel = "my-messages"`
        channel: String,
        /// `address = "redis://redis.example.com:6379"`
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
    }
}

/// Component reference or inline definition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged, try_from = "toml::Value")]
pub enum ComponentSpec {
    /// `"component-id"`
    Reference(KebabId),
    /// `{ ... }`
    Inline(Box<Component>),
}

impl TryFrom<toml::Value> for ComponentSpec {
    type Error = toml::de::Error;

    fn try_from(value: toml::Value) -> Result<Self, Self::Error> {
        if value.is_str() {
            Ok(ComponentSpec::Reference(KebabId::deserialize(value)?))
        } else {
            Ok(ComponentSpec::Inline(Box::new(Component::deserialize(
                value,
            )?)))
        }
    }
}

/// Component dependency
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ComponentDependency {
    /// `... = ">= 0.1.0"`
    Version(String),
    /// `... = { version = "0.1.0", registry = "registry.io", ...}`
    Package {
        /// Package version requirement
        version: String,
        /// Optional registry spec
        registry: Option<String>,
        /// Optional package name `foo:bar`. If not specified, the package name
        /// is inferred from the DependencyName key.
        package: Option<String>,
        /// Optional export name
        export: Option<String>,
    },
    /// `... = { path = "path/to/component.wasm", export = "my-export" }`
    Local {
        /// Path to Wasm
        path: PathBuf,
        /// Optional export name
        export: Option<String>,
    },
    /// `... = { url = "https://example.com/component.wasm", sha256 = "..." }`
    HTTP {
        /// URL to Wasm
        url: String,
        /// SHA256 Checksum of the component. The string should start with 'sha256:'
        digest: String,
        /// Optional export name
        export: Option<String>,
    },
}

/// Component definition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// `source = ...`
    pub source: ComponentSource,
    /// `description = "Component description"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// `variables = { name = "{{ app_var }}"}`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<LowerSnakeId, String>,
    /// `environment = { VAR = "value" }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub environment: Map<String, String>,
    /// `files = [...]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<WasiFilesMount>,
    /// `exclude_files = ["secrets/*"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_files: Vec<String>,
    /// `allowed_http_hosts = ["example.com"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[deprecated]
    pub allowed_http_hosts: Vec<String>,
    /// `allowed_outbound_hosts = ["redis://myredishost.com:6379"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_outbound_hosts: Vec<String>,
    /// `key_value_stores = ["default", "my-store"]`
    #[serde(
        default,
        with = "kebab_or_snake_case",
        skip_serializing_if = "Vec::is_empty"
    )]
    #[schemars(with = "Vec<String>")]
    pub key_value_stores: Vec<String>,
    /// `sqlite_databases = ["default", "my-database"]`
    #[serde(
        default,
        with = "kebab_or_snake_case",
        skip_serializing_if = "Vec::is_empty"
    )]
    #[schemars(with = "Vec<String>")]
    pub sqlite_databases: Vec<String>,
    /// `ai_models = ["llama2-chat"]`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ai_models: Vec<KebabId>,
    /// Build configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<ComponentBuildConfig>,
    /// Settings for custom tools or plugins. Spin ignores this field.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(schema_with = "map_of_toml_tables_schema")]
    pub tool: Map<String, toml::Table>,
    /// If true, allow dependencies to inherit configuration.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dependencies_inherit_configuration: bool,
    /// Component dependencies
    #[serde(default, skip_serializing_if = "ComponentDependencies::is_empty")]
    pub dependencies: ComponentDependencies,
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
    fn validate(&self) -> anyhow::Result<()> {
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
            if let DependencyName::Package(name) = dependency_name {
                if name.interface.is_none() {
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
        }
        Ok(())
    }

    /// This method ensures that dependencies names do not conflict with each other. That is to say
    /// that two dependencies of the same package must have disjoint versions or interfaces.
    fn ensure_disjoint(&self) -> anyhow::Result<()> {
        for (idx, this) in self.inner.keys().enumerate() {
            for other in self.inner.keys().skip(idx + 1) {
                let DependencyName::Package(other) = other else {
                    continue;
                };
                let DependencyName::Package(this) = this else {
                    continue;
                };

                if this.package == other.package {
                    Self::check_disjoint(this, other)?;
                }
            }
        }
        Ok(())
    }

    fn check_disjoint(
        this: &DependencyPackageName,
        other: &DependencyPackageName,
    ) -> anyhow::Result<()> {
        assert_eq!(this.package, other.package);

        if let (Some(this_ver), Some(other_ver)) = (this.version.clone(), other.version.clone()) {
            if Self::normalize_compatible_version(this_ver)
                != Self::normalize_compatible_version(other_ver)
            {
                return Ok(());
            }
        }

        if let (Some(this_itf), Some(other_itf)) =
            (this.interface.as_ref(), other.interface.as_ref())
        {
            if this_itf != other_itf {
                return Ok(());
            }
        }

        anyhow::bail!("{this:?} dependency conflicts with {other:?}")
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

mod kebab_or_snake_case {
    use serde::{Deserialize, Serialize};
    pub use spin_serde::{KebabId, SnakeId};
    pub fn serialize<S>(value: &[String], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        if value.iter().all(|s| {
            KebabId::try_from(s.clone()).is_ok() || SnakeId::try_from(s.to_owned()).is_ok()
        }) {
            value.serialize(serializer)
        } else {
            Err(serde::ser::Error::custom(
                "expected kebab-case or snake_case",
            ))
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = toml::Value::deserialize(deserializer)?;
        let list: Vec<String> = Vec::deserialize(value).map_err(serde::de::Error::custom)?;
        if list.iter().all(|s| {
            KebabId::try_from(s.clone()).is_ok() || SnakeId::try_from(s.to_owned()).is_ok()
        }) {
            Ok(list)
        } else {
            Err(serde::de::Error::custom(
                "expected kebab-case or snake_case",
            ))
        }
    }
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

mod one_or_many {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<T, S>(vec: &Vec<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize,
        S: Serializer,
    {
        if vec.len() == 1 {
            vec[0].serialize(serializer)
        } else {
            vec.serialize(serializer)
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
    where
        T: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let value = toml::Value::deserialize(deserializer)?;
        if let Ok(val) = T::deserialize(value.clone()) {
            Ok(vec![val])
        } else {
            Vec::deserialize(value).map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(test)]
mod tests {
    use toml::toml;

    use super::*;

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct FakeGlobalTriggerConfig {
        global_option: bool,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct FakeTriggerConfig {
        option: Option<bool>,
    }

    #[test]
    fn deserializing_trigger_configs() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [application.trigger.fake]
            global_option = true
            [[trigger.fake]]
            component = { source = "inline.wasm" }
            option = true
        })
        .unwrap();

        FakeGlobalTriggerConfig::deserialize(
            manifest.application.trigger_global_configs["fake"].clone(),
        )
        .unwrap();

        FakeTriggerConfig::deserialize(manifest.triggers["fake"][0].config.clone()).unwrap();
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct FakeGlobalToolConfig {
        lint_level: String,
    }

    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct FakeComponentToolConfig {
        command: String,
    }

    #[test]
    fn deserialising_custom_tool_settings() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [application.tool.lint]
            lint_level = "savage"
            [[trigger.fake]]
            something = "something else"
            [component.fake]
            source = "dummy"
            [component.fake.tool.clean]
            command = "cargo clean"
        })
        .unwrap();

        FakeGlobalToolConfig::deserialize(manifest.application.tool["lint"].clone()).unwrap();
        let fake_id: KebabId = "fake".to_owned().try_into().unwrap();
        FakeComponentToolConfig::deserialize(manifest.components[&fake_id].tool["clean"].clone())
            .unwrap();
    }

    #[test]
    fn deserializing_labels() {
        AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            something = "something else"
            [component.fake]
            source = "dummy"
            key_value_stores = ["default", "snake_case", "kebab-case"]
            sqlite_databases = ["default", "snake_case", "kebab-case"]
        })
        .unwrap();
    }

    #[test]
    fn deserializing_labels_fails_for_non_kebab_or_snake() {
        assert!(AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            something = "something else"
            [component.fake]
            source = "dummy"
            key_value_stores = ["b@dlabel"]
        })
        .is_err());
    }

    fn get_test_component_with_labels(labels: Vec<String>) -> Component {
        #[allow(deprecated)]
        Component {
            source: ComponentSource::Local("dummy".to_string()),
            description: "".to_string(),
            variables: Map::new(),
            environment: Map::new(),
            files: vec![],
            exclude_files: vec![],
            allowed_http_hosts: vec![],
            allowed_outbound_hosts: vec![],
            key_value_stores: labels.clone(),
            sqlite_databases: labels,
            ai_models: vec![],
            build: None,
            tool: Map::new(),
            dependencies_inherit_configuration: false,
            dependencies: Default::default(),
        }
    }

    #[test]
    fn serialize_labels() {
        let stores = vec![
            "default".to_string(),
            "snake_case".to_string(),
            "kebab-case".to_string(),
        ];
        let component = get_test_component_with_labels(stores.clone());
        let serialized = toml::to_string(&component).unwrap();
        let deserialized = toml::from_str::<Component>(&serialized).unwrap();
        assert_eq!(deserialized.key_value_stores, stores);
    }

    #[test]
    fn serialize_labels_fails_for_non_kebab_or_snake() {
        let component = get_test_component_with_labels(vec!["camelCase".to_string()]);
        assert!(toml::to_string(&component).is_err());
    }

    #[test]
    fn test_valid_snake_ids() {
        for valid in ["default", "mixed_CASE_words", "letters1_then2_numbers345"] {
            if let Err(err) = SnakeId::try_from(valid.to_string()) {
                panic!("{valid:?} should be value: {err:?}");
            }
        }
    }

    #[test]
    fn test_invalid_snake_ids() {
        for invalid in [
            "",
            "kebab-case",
            "_leading_underscore",
            "trailing_underscore_",
            "double__underscore",
            "1initial_number",
            "unicode_snowpeople☃☃☃",
            "mIxEd_case",
            "MiXeD_case",
        ] {
            if SnakeId::try_from(invalid.to_string()).is_ok() {
                panic!("{invalid:?} should not be a valid SnakeId");
            }
        }
    }

    #[test]
    fn test_check_disjoint() {
        for (a, b) in [
            ("foo:bar@0.1.0", "foo:bar@0.2.0"),
            ("foo:bar/baz@0.1.0", "foo:bar/baz@0.2.0"),
            ("foo:bar/baz@0.1.0", "foo:bar/bub@0.1.0"),
            ("foo:bar@0.1.0", "foo:bar/bub@0.2.0"),
            ("foo:bar@1.0.0", "foo:bar@2.0.0"),
            ("foo:bar@0.1.0", "foo:bar@1.0.0"),
            ("foo:bar/baz", "foo:bar/bub"),
            ("foo:bar/baz@0.1.0-alpha", "foo:bar/baz@0.1.0-beta"),
        ] {
            let a: DependencyPackageName = a.parse().expect(a);
            let b: DependencyPackageName = b.parse().expect(b);
            ComponentDependencies::check_disjoint(&a, &b).unwrap();
        }

        for (a, b) in [
            ("foo:bar@0.1.0", "foo:bar@0.1.1"),
            ("foo:bar/baz@0.1.0", "foo:bar@0.1.0"),
            ("foo:bar/baz@0.1.0", "foo:bar@0.1.0"),
            ("foo:bar", "foo:bar@0.1.0"),
            ("foo:bar@0.1.0-pre", "foo:bar@0.1.0-pre"),
        ] {
            let a: DependencyPackageName = a.parse().expect(a);
            let b: DependencyPackageName = b.parse().expect(b);
            assert!(
                ComponentDependencies::check_disjoint(&a, &b).is_err(),
                "{a} should conflict with {b}",
            );
        }
    }

    #[test]
    fn test_validate_dependencies() {
        // Specifying a dependency name as a plain-name without a package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "plain-name" = "0.1.0"
        })
        .unwrap()
        .validate()
        .is_err());

        // Specifying a dependency name as a plain-name without a package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "plain-name" = { version = "0.1.0" }
        })
        .unwrap()
        .validate()
        .is_err());

        // Specifying an export to satisfy a package dependency name is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:baz@0.1.0" = { path = "foo.wasm", export = "foo"}
        })
        .unwrap()
        .validate()
        .is_err());

        // Two compatible versions of the same package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:baz@0.1.0" = "0.1.0"
            "foo:bar@0.2.1" = "0.2.1"
            "foo:bar@0.2.2" = "0.2.2"
        })
        .unwrap()
        .validate()
        .is_err());

        // Two disjoint versions of the same package is ok
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar@0.1.0" = "0.1.0"
            "foo:bar@0.2.0" = "0.2.0"
            "foo:baz@0.2.0" = "0.1.0"
        })
        .unwrap()
        .validate()
        .is_ok());

        // Unversioned and versioned dependencies of the same package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar@0.1.0" = "0.1.0"
            "foo:bar" = ">= 0.2.0"
        })
        .unwrap()
        .validate()
        .is_err());

        // Two interfaces of two disjoint versions of a package is ok
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar/baz@0.1.0" = "0.1.0"
            "foo:bar/baz@0.2.0" = "0.2.0"
        })
        .unwrap()
        .validate()
        .is_ok());

        // A versioned interface and a different versioned package is ok
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar/baz@0.1.0" = "0.1.0"
            "foo:bar@0.2.0" = "0.2.0"
        })
        .unwrap()
        .validate()
        .is_ok());

        // A versioned interface and package of the same version is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar/baz@0.1.0" = "0.1.0"
            "foo:bar@0.1.0" = "0.1.0"
        })
        .unwrap()
        .validate()
        .is_err());

        // A versioned interface and unversioned package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar/baz@0.1.0" = "0.1.0"
            "foo:bar" = "0.1.0"
        })
        .unwrap()
        .validate()
        .is_err());

        // An unversioned interface and versioned package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar/baz" = "0.1.0"
            "foo:bar@0.1.0" = "0.1.0"
        })
        .unwrap()
        .validate()
        .is_err());

        // An unversioned interface and unversioned package is an error
        assert!(ComponentDependencies::deserialize(toml! {
            "foo:bar/baz" = "0.1.0"
            "foo:bar" = "0.1.0"
        })
        .unwrap()
        .validate()
        .is_err());
    }
}
