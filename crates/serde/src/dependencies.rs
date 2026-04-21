//! Types for working with component dependencies.

use crate::KebabId;
use anyhow::anyhow;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use wasm_pkg_common::package::PackageRef;

/// Name of an import package dependency.
///
/// For example: `foo:bar/baz@0.1.0`, `foo:bar/baz`, `foo:bar@0.1.0`, `foo:bar`.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(into = "String", try_from = "String")]
pub struct DependencyPackageName {
    /// The package spec, `foo:bar`, `foo:bar@0.1.0`.
    pub package: PackageRef,
    /// Package version
    pub version: Option<semver::Version>,
    /// Optional interface name.
    pub interface: Option<KebabId>,
}

impl std::fmt::Display for DependencyPackageName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.package)?;
        if let Some(interface) = &self.interface {
            write!(f, "/{interface}")?;
        }
        if let Some(version) = &self.version {
            write!(f, "@{version}")?;
        }
        Ok(())
    }
}

impl TryFrom<String> for DependencyPackageName {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<DependencyPackageName> for String {
    fn from(value: DependencyPackageName) -> Self {
        value.to_string()
    }
}

impl FromStr for DependencyPackageName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, version) = match s.split_once('@') {
            Some((name, version)) => (name, Some(version.parse()?)),
            None => (s, None),
        };

        let (package, interface) = match name.split_once('/') {
            Some((package, interface)) => (
                package.parse()?,
                Some(
                    interface
                        .to_string()
                        .try_into()
                        .map_err(|e| anyhow::anyhow!("{e}"))?,
                ),
            ),
            None => (name.parse()?, None),
        };

        Ok(DependencyPackageName {
            package,
            version,
            interface,
        })
    }
}

/// Name of an import dependency.
///
/// For example: `foo:bar/baz@0.1.0`, `foo:bar/baz`, `foo:bar@0.1.0`, `foo:bar`, `foo-bar`.
#[derive(
    Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Hash, JsonSchema,
)]
#[serde(into = "String", try_from = "String")]
#[schemars(with = "String")]
pub enum DependencyName {
    /// Plain name
    Plain(KebabId),
    /// Package spec
    Package(DependencyPackageName),
}

impl std::fmt::Display for DependencyName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            DependencyName::Plain(plain) => write!(f, "{plain}"),
            DependencyName::Package(name) => {
                write!(f, "{}", name.package)?;
                if let Some(interface) = &name.interface {
                    write!(f, "/{interface}")?;
                }
                if let Some(version) = &name.version {
                    write!(f, "@{version}")?;
                }
                Ok(())
            }
        }
    }
}

impl TryFrom<String> for DependencyName {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<DependencyName> for String {
    fn from(value: DependencyName) -> Self {
        value.to_string()
    }
}

impl FromStr for DependencyName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains([':', '/']) {
            Ok(Self::Package(s.parse()?))
        } else {
            Ok(Self::Plain(
                s.to_string().try_into().map_err(|e| anyhow!("{e}"))?,
            ))
        }
    }
}

impl DependencyName {
    /// Returns the package reference if this is a package dependency name.
    pub fn package(&self) -> Option<&PackageRef> {
        match self {
            DependencyName::Package(name) => Some(&name.package),
            DependencyName::Plain(_) => None,
        }
    }
}
