//! Types for working with component dependencies.

use crate::KebabId;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Name of an import package dependency.
///
/// For example: `foo:bar/baz@0.1.0`, `foo:bar/baz`, `foo:bar@0.1.0`, `foo:bar`.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
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

/// A package reference, consisting of kebab-case namespace and name.
///
/// Ex: `wasm-pkg:client`
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(into = "String", try_from = "String")]
pub struct PackageRef {
    /// The package namespace
    pub namespace: String,
    /// The package name
    pub name: String,
}

impl PackageRef {
    /// Returns the package namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the package name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for PackageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.name)
    }
}

impl From<PackageRef> for String {
    fn from(value: PackageRef) -> Self {
        value.to_string()
    }
}

impl TryFrom<String> for PackageRef {
    type Error = anyhow::Error;

    fn try_from(mut value: String) -> Result<Self, Self::Error> {
        let Some(colon) = value.find(':') else {
            anyhow::bail!("missing expected ':'");
        };
        let name = value.split_off(colon + 1);
        value.truncate(colon);
        Ok(Self {
            // TODO(rylev): parse both fields as labels
            namespace: value,
            name: name,
        })
    }
}

impl FromStr for PackageRef {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.to_string().try_into()
    }
}

/// A registry identifier.
///
/// This must be a valid HTTP Host.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Registry(http::uri::Authority);

impl Registry {
    /// Returns the registry host, without port number.
    pub fn host(&self) -> &str {
        self.0.host()
    }

    /// Returns the registry port number, if given.
    pub fn port(&self) -> Option<u16> {
        self.0.port_u16()
    }
}

impl AsRef<str> for Registry {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Registry> for String {
    fn from(value: Registry) -> Self {
        value.to_string()
    }
}

impl std::str::FromStr for Registry {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

impl TryFrom<String> for Registry {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(value.try_into()?))
    }
}

/// Name of an import dependency.
///
/// For example: `foo:bar/baz@0.1.0`, `foo:bar/baz`, `foo:bar@0.1.0`, `foo:bar`, `foo-bar`.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(into = "String", try_from = "String")]
pub enum DependencyName {
    /// Plain name
    Plain(KebabId),
    /// Package spec
    Package(DependencyPackageName),
}

// TODO: replace with derive once wasm-pkg-common is released
impl PartialOrd for DependencyName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// TODO: replace with derive once wasm-pkg-common is released
impl Ord for DependencyName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (DependencyName::Plain(a), DependencyName::Plain(b)) => a.cmp(b),
            (DependencyName::Package(a), DependencyName::Package(b)) => {
                let big_ole_tup = (
                    a.package.namespace(),
                    a.package.name(),
                    a.interface.as_ref(),
                    a.version.as_ref(),
                );
                let other_big_ole_tup = (
                    b.package.namespace().as_ref(),
                    b.package.name().as_ref(),
                    b.interface.as_ref(),
                    b.version.as_ref(),
                );
                big_ole_tup.cmp(&other_big_ole_tup)
            }
            (DependencyName::Plain(_), DependencyName::Package(_)) => std::cmp::Ordering::Less,
            (DependencyName::Package(_), DependencyName::Plain(_)) => std::cmp::Ordering::Greater,
        }
    }
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
