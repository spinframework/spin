//! Publishing standalone Wasm components to a registry.

use std::path::Path;

use anyhow::{Context, Result};
use wasm_pkg_client::{Client, PackageRef, PublishOpts, Registry, Version};

/// Publishes a standalone Wasm component to a registry as a wasm-pkg component
/// package, so that other Spin applications can consume it as a registry
/// component dependency.
///
/// The `name` and `version` come from the component manifest. The package
/// namespace is derived from `registry`: the last path segment is the namespace
/// and the leading segment is the registry host (for example `ghcr.io/example`
/// publishes `example:<name>` to `ghcr.io`). Together these form the wasm-pkg
/// package reference `namespace:name@version`. Returns the
/// `namespace:name@version` that was published.
pub async fn publish_component(
    name: &str,
    version: &str,
    registry: &str,
    wasm_path: impl AsRef<Path>,
) -> Result<String> {
    let (package, version, registry) = component_package_ref(name, version, registry)?;

    let client = Client::with_global_defaults()
        .await
        .context("failed to initialize the wasm-pkg registry client")?;

    client
        .publish_release_file(
            wasm_path.as_ref(),
            PublishOpts {
                package: Some((package.clone(), version.clone())),
                registry: Some(registry),
            },
        )
        .await
        .with_context(|| format!("failed to publish component {package}@{version}"))?;

    Ok(format!("{package}@{version}"))
}

/// Builds a wasm-pkg package reference, version, and registry host from the
/// `name` and `version` declared in a component manifest and the `registry`
/// supplied on the command line.
///
/// The package namespace is the last `/`-separated segment of `registry`; the
/// registry host is the first segment. For example `ghcr.io/example` yields
/// package `example:<name>` published to host `ghcr.io`.
fn component_package_ref(
    name: &str,
    version: &str,
    registry: &str,
) -> Result<(PackageRef, Version, Registry)> {
    anyhow::ensure!(
        !name.is_empty(),
        "no name specified for the component; set `name` in the `[component]` section of \
         the component manifest"
    );
    anyhow::ensure!(
        registry.contains('/'),
        "cannot derive a package namespace from registry {registry:?}; include the namespace \
         as the last path segment (for example `ghcr.io/my-namespace`)"
    );

    let host = registry.split('/').next().unwrap();
    let namespace = registry.rsplit('/').next().unwrap();

    let registry = host.parse::<Registry>().with_context(|| {
        format!("invalid registry host {host:?} derived from registry {registry:?}")
    })?;

    let package_str = format!("{namespace}:{name}");
    let package = package_str.parse::<PackageRef>().with_context(|| {
        format!("invalid component package reference {package_str:?} (expected `namespace:name`)")
    })?;

    anyhow::ensure!(
        !version.is_empty(),
        "no version specified for component {package_str}; set `version` in the `[component]` \
         section of the component manifest (for example `version = \"1.0.0\"`)"
    );
    let version = version.parse::<Version>().with_context(|| {
        format!("invalid component version {version:?} (expected a semver version, for example 1.0.0)")
    })?;

    Ok((package, version, registry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_package_ref_from_name_and_registry() {
        let (package, version, registry) =
            component_package_ref("comp", "1.2.3", "ghcr.io/example").unwrap();
        assert_eq!(package.to_string(), "example:comp");
        assert_eq!(version.to_string(), "1.2.3");
        assert_eq!(registry.to_string(), "ghcr.io");
    }

    #[test]
    fn derives_namespace_from_last_registry_segment() {
        let (package, _version, registry) =
            component_package_ref("comp", "1.2.3", "ghcr.io/michellen/spin-components").unwrap();
        assert_eq!(package.to_string(), "spin-components:comp");
        assert_eq!(registry.to_string(), "ghcr.io");
    }

    #[test]
    fn rejects_registry_without_namespace_segment() {
        assert!(component_package_ref("comp", "1.0.0", "ghcr.io").is_err());
    }

    #[test]
    fn rejects_missing_name() {
        assert!(component_package_ref("", "1.0.0", "ghcr.io/example").is_err());
    }

    #[test]
    fn rejects_missing_version() {
        assert!(component_package_ref("comp", "", "ghcr.io/example").is_err());
    }

    #[test]
    fn rejects_invalid_package() {
        assert!(component_package_ref("comp", "1.0.0", "ghcr.io/not a namespace").is_err());
    }
}
