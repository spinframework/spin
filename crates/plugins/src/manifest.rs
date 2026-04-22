use std::io::IsTerminal;

use anyhow::{Context, Result, anyhow};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::PluginStore;

/// Expected schema of a plugin manifest. Should match the latest Spin plugin
/// manifest JSON schema:
/// <https://github.com/spinframework/spin-plugins/tree/main/json-schema>
#[derive(Serialize, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    /// Name of the plugin.
    name: String,
    /// Option description of the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    /// Optional address to the homepage of the plugin producer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    homepage: Option<String>,
    /// Version of the plugin.
    pub(crate) version: String,
    /// Versions of Spin that the plugin is compatible with.
    pub(crate) spin_compatibility: String,
    /// License of the plugin.
    license: String,
    /// Points to source package[s] of the plugin..
    pub(crate) packages: Vec<PluginPackage>,
}

impl PluginManifest {
    pub fn name(&self) -> String {
        self.name.to_lowercase()
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn license(&self) -> &str {
        self.license.as_ref()
    }

    pub fn spin_compatibility(&self) -> String {
        self.spin_compatibility.clone()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn homepage_url(&self) -> Option<Url> {
        Url::parse(self.homepage.as_deref()?).ok()
    }

    pub fn has_compatible_package(&self) -> bool {
        self.packages.iter().any(|p| p.matches_current_os_arch())
    }
    pub fn is_compatible_spin_version(&self, spin_version: &str) -> bool {
        is_version_compatible_enough(&self.spin_compatibility, spin_version).unwrap_or(false)
    }
    pub fn is_installed_in(&self, store: &PluginStore) -> bool {
        match store.read_plugin_manifest(&self.name) {
            Ok(m) => m.eq(self),
            Err(_) => false,
        }
    }

    pub fn try_version(&self) -> Result<semver::Version, semver::Error> {
        semver::Version::parse(&self.version)
    }

    // Compares the versions. Returns None if either's version string is invalid semver.
    pub fn compare_versions(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if let Ok(this_version) = self.try_version()
            && let Ok(other_version) = other.try_version()
        {
            return Some(this_version.cmp_precedence(&other_version));
        }
        None
    }
}

/// Describes compatibility and location of a plugin source.
#[derive(Serialize, Debug, Deserialize, PartialEq)]
pub struct PluginPackage {
    /// Compatible OS.
    pub(crate) os: Os,
    /// Compatible architecture.
    pub(crate) arch: Architecture,
    /// Address to fetch the plugin source tar file.
    pub(crate) url: String,
    /// Checksum to verify the plugin before installation.
    pub(crate) sha256: String,
}

impl PluginPackage {
    pub fn url(&self) -> String {
        self.url.clone()
    }
    pub fn matches_current_os_arch(&self) -> bool {
        self.os.rust_name() == std::env::consts::OS
            && self.arch.rust_name() == std::env::consts::ARCH
    }
}

/// Describes the compatible OS of a plugin
#[derive(Serialize, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Os {
    Linux,
    Macos,
    Windows,
}

impl Os {
    // Maps manifest OS options to associated Rust OS strings
    // https://doc.rust-lang.org/std/env/consts/constant.OS.html
    pub(crate) fn rust_name(&self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }
}

/// Describes the compatible architecture of a plugin
#[derive(Serialize, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Architecture {
    Amd64,
    Aarch64,
    Arm,
}

impl Architecture {
    // Maps manifest Architecture options to associated Rust ARCH strings
    // https://doc.rust-lang.org/std/env/consts/constant.ARCH.html
    pub(crate) fn rust_name(&self) -> &'static str {
        match self {
            Architecture::Amd64 => "x86_64",
            Architecture::Aarch64 => "aarch64",
            Architecture::Arm => "arm",
        }
    }
}

/// Checks whether the plugin supports the currently running version of Spin
/// and prints a warning if not (or if uncertain).
pub fn warn_unsupported_version(
    manifest: &PluginManifest,
    spin_version: &str,
    override_compatibility_check: bool,
) -> Result<()> {
    let supported_on = &manifest.spin_compatibility;
    inner_warn_unsupported_version(supported_on, spin_version, override_compatibility_check)
}

/// Does the manifest compatibility pattern match this version of Spin?  This is a
/// strict semver check.
fn is_version_fully_compatible(supported_on: &str, spin_version: &str) -> Result<bool> {
    let comparator = VersionReq::parse(supported_on).with_context(|| {
        format!("Could not parse manifest compatibility version {supported_on} as valid semver")
    })?;
    let version = Version::parse(spin_version)?;
    Ok(comparator.matches(&version))
}

/// This is more liberal than `is_version_fully_compatible`; it relaxes the semver requirement
/// for Spin pre-releases, so that you don't get *every* plugin showing as incompatible when
/// you run a pre-release.  This is intended for listing; when executing, we use the interactive
/// `warn_unsupported_version`, which provides the full nuanced feedback.
pub(crate) fn is_version_compatible_enough(supported_on: &str, spin_version: &str) -> Result<bool> {
    if is_version_fully_compatible(supported_on, spin_version)? {
        Ok(true)
    } else {
        // We allow things to run on pre-release versions, because otherwise EVERYTHING would
        // show as incompatible!
        let is_spin_prerelease = Version::parse(spin_version)
            .map(|v| !v.pre.is_empty())
            .unwrap_or_default();
        Ok(is_spin_prerelease)
    }
}

fn inner_warn_unsupported_version(
    supported_on: &str,
    spin_version: &str,
    override_compatibility_check: bool,
) -> Result<()> {
    if !is_version_fully_compatible(supported_on, spin_version)? {
        let show_warnings = !suppress_compatibility_warnings();
        let version = Version::parse(spin_version)?;
        if !version.pre.is_empty() {
            if std::io::stderr().is_terminal() && show_warnings {
                terminal::warn!(
                    "You're using a pre-release version of Spin ({spin_version}). This plugin might not be compatible (supported: {supported_on}). Continuing anyway."
                );
            }
        } else if override_compatibility_check {
            if show_warnings {
                terminal::warn!(
                    "Plugin is not compatible with this version of Spin (supported: {supported_on}, actual: {spin_version}). Check overridden ... continuing to install or execute plugin."
                );
            }
        } else {
            return Err(anyhow!(
                "Plugin is not compatible with this version of Spin (supported: {supported_on}, actual: {spin_version}). Try running `spin plugins update && spin plugins upgrade --all` to install latest or override with `--override-compatibility-check`."
            ));
        }
    }
    Ok(())
}

fn suppress_compatibility_warnings() -> bool {
    match std::env::var("SPIN_PLUGINS_SUPPRESS_COMPATIBILITY_WARNINGS") {
        Ok(s) => !s.is_empty(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn generate_test_manifest(
        name: &str,
        version: &str,
        license: &str,
        description: Option<&str>,
        homepage: Option<&str>,
    ) -> PluginManifest {
        let mut plugin_json = serde_json::json!(
        {
            "name": name,
            "version": version,
            "spinCompatibility": "=0.4",
            "license": license,
            "packages": [
                {
                    "os": "linux",
                    "arch": "amd64",
                    "url": "www.example.com/releases/1.0/binary.tgz",
                    "sha256": "c474f00b12345e38acae2d19b2a707a4fhdjdfdd22875efeefdf052ce19c90b"
                },
                {
                    "os": "windows",
                    "arch": "amd64",
                    "url": "www.example.com/releases/1.0/binary.tgz",
                    "sha256": "eee4f00b12345e38acae2d19b2a707a4fhdjdfdd22875efeefdf052ce19c90b"
                },
                {
                    "os": "macos",
                    "arch": "aarch64",
                    "url": "www.example.com/releases/1.0/binary.tgz",
                    "sha256": "eeegf00b12345e38acae2d19b2a707a4fhdjdfdd22875efeefdf052ce19c90b"
                }
            ]
        });
        if let Some(homepage) = homepage {
            plugin_json
                .as_object_mut()
                .unwrap()
                .insert("homepage".to_string(), serde_json::json!(homepage));
        }
        if let Some(description) = description {
            plugin_json
                .as_object_mut()
                .unwrap()
                .insert("description".to_string(), serde_json::json!(description));
        }
        serde_json::from_value(plugin_json).unwrap()
    }

    #[test]
    fn test_supported_version() {
        let test_case = ">=1.2.3, <1.8.0";
        let input_output = [
            ("1.3.0", true),
            ("1.2.3", true),
            ("1.8.0", false),
            ("1.9.0", false),
            ("1.2.0", false),
        ];
        input_output.into_iter().for_each(|(i, o)| {
            assert_eq!(
                inner_warn_unsupported_version(test_case, i, false).is_ok(),
                o
            )
        });
    }

    #[test]
    fn test_plugin_json() {
        let name = "test";
        let description = "Some description.";
        let homepage = "www.example.com";
        let version = "1.0";
        let license = "Mit";
        let deserialized_plugin =
            generate_test_manifest(name, version, license, Some(description), Some(homepage));
        assert_eq!(deserialized_plugin.name(), name.to_owned());
        assert_eq!(
            deserialized_plugin.description,
            Some(description.to_owned())
        );
        assert_eq!(deserialized_plugin.homepage, Some(homepage.to_owned()));
        assert_eq!(deserialized_plugin.version, version.to_owned());
        assert_eq!(deserialized_plugin.license, license.to_owned());
        assert_eq!(deserialized_plugin.packages.len(), 3);
    }

    #[test]
    fn test_plugin_json_empty_options() {
        let deserialized_plugin = generate_test_manifest("name", "0.1.1", "Mit", None, None);
        assert_eq!(deserialized_plugin.description, None);
        assert_eq!(deserialized_plugin.homepage, None);
    }
}
