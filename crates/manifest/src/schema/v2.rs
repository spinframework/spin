use anyhow::{Context, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use spin_serde::{DependencyName, DependencyPackageName, FixedVersion, LowerSnakeId};
pub use spin_serde::{KebabId, SnakeId};

pub use super::common::{ComponentBuildConfig, ComponentSource, Variable, WasiFilesMount};
use super::{json_schema, kebab_or_snake_case, one_or_many};

pub(crate) type Map<K, V> = indexmap::IndexMap<K, V>;

mod app_details;
mod component;
mod dependency;
mod target_env;
mod trigger;

pub use app_details::AppDetails;
pub use component::{
    Component, ComponentDependencies, ComponentProfileBuildOverride, ComponentProfileOverride,
};
pub use dependency::{ComponentDependency, InheritConfiguration, TriggerDependency};
pub use target_env::TargetEnvironmentRef;
pub use trigger::{ComponentSpec, OneOrManyComponentSpecs, Trigger, TriggerDependencies};

/// App manifest
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    /// `spin_manifest_version = 2`
    #[schemars(with = "usize", range(min = 2, max = 2))]
    pub spin_manifest_version: FixedVersion<2>,
    /// `[application]`
    pub application: AppDetails,
    /// Application configuration variables. These can be set via environment variables, or
    /// from sources such as Hashicorp Vault or Azure KeyVault by using a runtime config file.
    /// They are not available directly to components: use a component variable to ingest them.
    ///
    /// Learn more: https://spinframework.dev/variables, https://spinframework.dev/dynamic-configuration#application-variables-runtime-configuration
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub variables: Map<LowerSnakeId, Variable>,
    /// The triggers to which the application responds. Most triggers can appear
    /// multiple times with different parameters: for example, the `http` trigger may
    /// appear multiple times with different routes, or the `redis` trigger with
    /// different channels.
    ///
    /// Example: `[[trigger.http]]`
    #[serde(rename = "trigger")]
    #[schemars(with = "json_schema::TriggerSchema")]
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

    /// Whether any component in the application defines the given profile.
    /// Not every component defines every profile, and components intentionally
    /// fall back to the anonymouse profile if they are asked for a profile
    /// they don't define. So this can be used to detect that a user might have
    /// mistyped a profile (e.g. `spin up --profile deugb`).
    pub fn ensure_profile(&self, profile: Option<&str>) -> anyhow::Result<()> {
        let Some(p) = profile else {
            return Ok(());
        };

        let is_defined = self.components.values().any(|c| c.profile.contains_key(p));

        if is_defined {
            Ok(())
        } else {
            Err(anyhow!("Profile {p} is not defined in this application"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
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

    fn as_reference(spec: &ComponentSpec) -> Option<&str> {
        match spec {
            ComponentSpec::Reference(id) => Some(id.as_ref()),
            ComponentSpec::Inline(_) => None,
        }
    }

    fn as_inline(spec: &ComponentSpec) -> Option<&Component> {
        match spec {
            ComponentSpec::Reference(_) => None,
            ComponentSpec::Inline(c) => Some(c),
        }
    }

    fn as_local(source: &ComponentSource) -> Option<&str> {
        match source {
            ComponentSource::Local(path) => Some(path),
            _ => None,
        }
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
        assert!(
            AppManifest::deserialize(toml! {
                spin_manifest_version = 2
                [application]
                name = "trigger-configs"
                [[trigger.fake]]
                something = "something else"
                [component.fake]
                source = "dummy"
                key_value_stores = ["b@dlabel"]
            })
            .is_err()
        );
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
            targets: None,
            build: None,
            tool: Map::new(),
            dependencies_inherit_configuration: None,
            dependencies: Default::default(),
            profile: Default::default(),
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
        assert!(
            ComponentDependencies::deserialize(toml! {
                "plain-name" = "0.1.0"
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // Specifying a dependency name as a plain-name without a package is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "plain-name" = { version = "0.1.0" }
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // Specifying an export to satisfy a package dependency name is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:baz@0.1.0" = { path = "foo.wasm", export = "foo"}
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // Two compatible versions of the same package is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:baz@0.1.0" = "0.1.0"
                "foo:bar@0.2.1" = "0.2.1"
                "foo:bar@0.2.2" = "0.2.2"
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // Two disjoint versions of the same package is ok
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar@0.1.0" = "0.1.0"
                "foo:bar@0.2.0" = "0.2.0"
                "foo:baz@0.2.0" = "0.1.0"
            })
            .unwrap()
            .validate()
            .is_ok()
        );

        // Unversioned and versioned dependencies of the same package is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar@0.1.0" = "0.1.0"
                "foo:bar" = ">= 0.2.0"
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // Two interfaces of two disjoint versions of a package is ok
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar/baz@0.1.0" = "0.1.0"
                "foo:bar/baz@0.2.0" = "0.2.0"
            })
            .unwrap()
            .validate()
            .is_ok()
        );

        // A versioned interface and a different versioned package is ok
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar/baz@0.1.0" = "0.1.0"
                "foo:bar@0.2.0" = "0.2.0"
            })
            .unwrap()
            .validate()
            .is_ok()
        );

        // A versioned interface and package of the same version is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar/baz@0.1.0" = "0.1.0"
                "foo:bar@0.1.0" = "0.1.0"
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // A versioned interface and unversioned package is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar/baz@0.1.0" = "0.1.0"
                "foo:bar" = "0.1.0"
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // An unversioned interface and versioned package is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar/baz" = "0.1.0"
                "foo:bar@0.1.0" = "0.1.0"
            })
            .unwrap()
            .validate()
            .is_err()
        );

        // An unversioned interface and unversioned package is an error
        assert!(
            ComponentDependencies::deserialize(toml! {
                "foo:bar/baz" = "0.1.0"
                "foo:bar" = "0.1.0"
            })
            .unwrap()
            .validate()
            .is_err()
        );
    }

    fn normalized_component(
        manifest: &AppManifest,
        component: &str,
        profile: Option<&str>,
    ) -> Component {
        use crate::normalize::normalize_manifest;

        let id =
            KebabId::try_from(component.to_owned()).expect("component ID should have been kebab");

        let mut manifest = manifest.clone();
        normalize_manifest(&mut manifest, profile).expect("should have normalised");
        manifest
            .components
            .get(&id)
            .expect("should have compopnent with id profile-test")
            .clone()
    }

    #[test]
    fn profiles_override_source() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            component = "profile-test"
            [component.profile-test]
            source = "original"
            [component.profile-test.profile.fancy]
            source = "fancy-schmancy"
        })
        .expect("manifest should be valid");

        let id = "profile-test";

        let component = normalized_component(&manifest, id, None);
        assert!(matches!(&component.source, ComponentSource::Local(p) if p == "original"));

        let component = normalized_component(&manifest, id, Some("fancy"));
        assert!(matches!(&component.source, ComponentSource::Local(p) if p == "fancy-schmancy"));

        let component = normalized_component(&manifest, id, Some("non-existent"));
        assert!(matches!(&component.source, ComponentSource::Local(p) if p == "original"));
    }

    #[test]
    fn profiles_override_build_command() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            component = "profile-test"
            [component.profile-test]
            source = "original"
            build.command = "buildme --release"
            [component.profile-test.profile.fancy]
            source = "fancy-schmancy"
            build.command = ["buildme --fancy", "lintme"]
        })
        .expect("manifest should be valid");

        let id = "profile-test";

        let build = normalized_component(&manifest, id, None)
            .build
            .expect("should have default build");
        assert_eq!(1, build.commands().len());
        assert_eq!("buildme --release", build.commands().next().unwrap());

        let build = normalized_component(&manifest, id, Some("fancy"))
            .build
            .expect("should have fancy build");
        assert_eq!(2, build.commands().len());
        assert_eq!("buildme --fancy", build.commands().next().unwrap());
        assert_eq!("lintme", build.commands().nth(1).unwrap());

        let build = normalized_component(&manifest, id, Some("non-existent"))
            .build
            .expect("should fall back to default build");
        assert_eq!(1, build.commands().len());
        assert_eq!("buildme --release", build.commands().next().unwrap());
    }

    #[test]
    fn profiles_can_have_build_command_when_default_doesnt() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            component = "profile-test"
            [component.profile-test]
            source = "original"
            [component.profile-test.profile.fancy]
            source = "fancy-schmancy"
            build.command = ["buildme --fancy", "lintme"]
        })
        .expect("manifest should be valid");

        let component = normalized_component(&manifest, "profile-test", None);
        assert!(component.build.is_none(), "shouldn't have default build");

        let component = normalized_component(&manifest, "profile-test", Some("fancy"));
        assert!(component.build.is_some(), "should have fancy build");

        let build = component.build.expect("should have fancy build");

        assert_eq!(2, build.commands().len());
        assert_eq!("buildme --fancy", build.commands().next().unwrap());
        assert_eq!("lintme", build.commands().nth(1).unwrap());
    }

    #[test]
    fn profiles_override_env_vars() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            component = "profile-test"
            [component.profile-test]
            source = "original"
            environment = { DB_URL = "pg://production" }
            [component.profile-test.profile.fancy]
            environment = { DB_URL = "pg://fancy", FANCINESS = "1" }
        })
        .expect("manifest should be valid");

        let id = "profile-test";

        let component = normalized_component(&manifest, id, None);

        assert_eq!(1, component.environment.len());
        assert_eq!(
            "pg://production",
            component
                .environment
                .get("DB_URL")
                .expect("DB_URL should have been set")
        );

        let component = normalized_component(&manifest, id, Some("fancy"));

        assert_eq!(2, component.environment.len());
        assert_eq!(
            "pg://fancy",
            component
                .environment
                .get("DB_URL")
                .expect("DB_URL should have been set")
        );
        assert_eq!(
            "1",
            component
                .environment
                .get("FANCINESS")
                .expect("FANCINESS should have been set")
        );
    }

    #[test]
    fn profiles_dependencies() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "trigger-configs"
            [[trigger.fake]]
            component = "profile-test"
            [component.profile-test]
            source = "original"
            [component.profile-test.dependencies]
            "foo-bar" = "1.0.0"
            [component.profile-test.profile.fancy]
            dependencies = { "foo-bar" = { path = "local.wasm" }, "fancy-thing" = "1.2.3" }
        })
        .expect("manifest should be valid");

        let id = "profile-test";

        let component = normalized_component(&manifest, id, None);

        assert_eq!(1, component.dependencies.inner.len());
        assert!(matches!(
            component
                .dependencies
                .inner
                .get(&DependencyName::Plain(KebabId::try_from("foo-bar".to_owned()).unwrap()))
                .expect("foo-bar dep should have been set"),
            ComponentDependency::Version(v) if v == "1.0.0",
        ));

        let component = normalized_component(&manifest, id, Some("fancy"));

        assert_eq!(2, component.dependencies.inner.len());
        assert!(matches!(
            component
                .dependencies
                .inner
                .get(&DependencyName::Plain(KebabId::try_from("foo-bar".to_owned()).unwrap()))
                .expect("foo-bar dep should have been set"),
            ComponentDependency::Local { path, .. } if path == &PathBuf::from("local.wasm"),
        ));
        assert!(matches!(
            component
                .dependencies
                .inner
                .get(&DependencyName::Plain(KebabId::try_from("fancy-thing".to_owned()).unwrap()))
                .expect("fancy-thing dep should have been set"),
            ComponentDependency::Version(v) if v == "1.2.3",
        ));
    }

    #[test]
    fn can_deserialise_one_or_many_one_ref() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "test"
            [[trigger.fake]]
            component = "test1"
            components = { babble = "test2" }
        })
        .expect("manifest should be valid");

        let trigger = manifest.triggers.get("fake").unwrap()[0].clone();

        assert_eq!(
            Some("test1"),
            as_reference(trigger.component.as_ref().unwrap())
        );
        assert_eq!(1, trigger.components.len());
        let babble_comps = &trigger.components.get("babble").as_ref().unwrap().0;
        assert_eq!(1, babble_comps.len());
        assert_eq!(Some("test2"), as_reference(&babble_comps[0]));
    }

    #[test]
    fn can_deserialise_one_or_many_one_inline() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "test"
            [[trigger.fake]]
            component = "test1"
            components = { babble = { source = "fie.wasm", allowed_outbound_hosts = ["http://example.com"] } }
        })
        .expect("manifest should be valid");

        let trigger = manifest.triggers.get("fake").unwrap()[0].clone();

        assert_eq!(1, trigger.components.len());
        let babble_comps = &trigger.components.get("babble").as_ref().unwrap().0;
        assert_eq!(1, babble_comps.len());
        let single = as_inline(&babble_comps[0]).expect("should have deserialised to inline");
        assert_eq!(Some("fie.wasm"), as_local(&single.source));
        assert_eq!(1, single.allowed_outbound_hosts.len());
    }

    #[test]
    fn can_deserialise_one_or_many_many() {
        let manifest = AppManifest::deserialize(toml! {
            spin_manifest_version = 2
            [application]
            name = "test"
            [[trigger.fake]]
            component = "test1"
            components = { babble = ["test2", { source = "fie.wasm", allowed_outbound_hosts = ["http://example.com"] }, "test3"] }
        })
        .expect("manifest should be valid");

        let trigger = manifest.triggers.get("fake").unwrap()[0].clone();

        assert_eq!(1, trigger.components.len());
        let babble_comps = &trigger.components.get("babble").as_ref().unwrap().0;
        assert_eq!(3, babble_comps.len());

        assert_eq!(Some("test2"), as_reference(&babble_comps[0]));

        let inline = as_inline(&babble_comps[1]).expect("should have deserialised to inline");
        assert_eq!(Some("fie.wasm"), as_local(&inline.source));
        assert_eq!(1, inline.allowed_outbound_hosts.len());

        assert_eq!(Some("test3"), as_reference(&babble_comps[2]));
    }
}
