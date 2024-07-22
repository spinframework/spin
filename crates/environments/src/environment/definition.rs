//! Environment definition types and serialisation (TOML) formats
//!
//! This module does *not* cover loading those definitions from remote
//! sources, or materialising WIT packages from files or registry references -
//! only the types.

use std::collections::HashMap;

use anyhow::Context;

/// An environment definition, usually deserialised from a TOML document.
/// Example:
///
/// ```ignore
/// # spin-up.3.2.toml
/// [triggers]
/// http = ["spin:up/http-trigger@3.2.0", "spin:up/http-trigger-rc20231018@3.2.0"]
/// redis = ["spin:up/redis-trigger@3.2.0"]
/// ```
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDefinition {
    triggers: HashMap<String, Vec<WorldRef>>,
    default: Option<Vec<WorldRef>>,
}

impl EnvironmentDefinition {
    pub fn triggers(&self) -> &HashMap<String, Vec<WorldRef>> {
        &self.triggers
    }

    pub fn default(&self) -> Option<&Vec<WorldRef>> {
        self.default.as_ref()
    }
}

/// A reference to a world in an [EnvironmentDefinition]. This is formed
/// of a fully qualified (ns:pkg/id) world name, optionally with
/// a location from which to get the package (a registry or WIT directory).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum WorldRef {
    DefaultRegistry(WorldName),
    Registry {
        registry: String,
        world: WorldName,
    },
    WitDirectory {
        path: std::path::PathBuf,
        world: WorldName,
    },
}

/// The qualified name of a world, e.g. spin:up/http-trigger@3.2.0.
///
/// (Internally it is represented as a PackageName plus unqualified
/// world name, but it stringises to the standard WIT qualified name.)
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct WorldName {
    package: wit_parser::PackageName,
    world: String,
}

impl WorldName {
    pub fn package(&self) -> &wit_parser::PackageName {
        &self.package
    }

    pub fn package_namespaced_name(&self) -> String {
        format!("{}:{}", self.package.namespace, self.package.name)
    }

    pub fn package_ref(&self) -> anyhow::Result<wasm_pkg_client::PackageRef> {
        let pkg_name = self.package_namespaced_name();
        pkg_name
            .parse()
            .with_context(|| format!("Environment {pkg_name} is not a valid package name"))
    }

    pub fn package_version(&self) -> Option<&semver::Version> {
        self.package.version.as_ref()
    }
}

impl TryFrom<String> for WorldName {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        use wasmparser::names::{ComponentName, ComponentNameKind};

        // World qnames have the same syntactic form as interface qnames
        let parsed = ComponentName::new(&value, 0)?;
        let ComponentNameKind::Interface(itf) = parsed.kind() else {
            anyhow::bail!("{value} is not a well-formed world name");
        };

        let package = wit_parser::PackageName {
            namespace: itf.namespace().to_string(),
            name: itf.package().to_string(),
            version: itf.version(),
        };

        let world = itf.interface().to_string();

        Ok(Self { package, world })
    }
}

impl std::fmt::Display for WorldName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.package.namespace)?;
        f.write_str(":")?;
        f.write_str(&self.package.name)?;
        f.write_str("/")?;
        f.write_str(&self.world)?;

        if let Some(v) = self.package.version.as_ref() {
            f.write_str("@")?;
            f.write_str(&v.to_string())?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_parse_versioned_world_name() {
        let text = "ns:name/world@1.0.0";
        let w = WorldName::try_from(text.to_owned()).unwrap();

        assert_eq!("ns", w.package().namespace);
        assert_eq!("name", w.package().name);
        assert_eq!("ns:name", w.package_namespaced_name());
        assert_eq!("ns", w.package_ref().unwrap().namespace().to_string());
        assert_eq!("name", w.package_ref().unwrap().name().to_string());
        assert_eq!("world", w.world);
        assert_eq!(
            &semver::Version::new(1, 0, 0),
            w.package().version.as_ref().unwrap()
        );

        assert_eq!(text, w.to_string());
    }

    #[test]
    fn can_parse_unversioned_world_name() {
        let text = "ns:name/world";
        let w = WorldName::try_from("ns:name/world".to_owned()).unwrap();

        assert_eq!("ns", w.package().namespace);
        assert_eq!("name", w.package().name);
        assert_eq!("ns:name", w.package_namespaced_name());
        assert_eq!("ns", w.package_ref().unwrap().namespace().to_string());
        assert_eq!("name", w.package_ref().unwrap().name().to_string());
        assert_eq!("world", w.world);
        assert!(w.package().version.is_none());

        assert_eq!(text, w.to_string());
    }
}
