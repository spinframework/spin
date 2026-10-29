//! Parsing and resolution of the dependency source given to `spin dependencies add`.

use anyhow::{Context, Result, anyhow, bail};
use spin_manifest::schema::v2::{AppManifest, ComponentDependency, InheritConfiguration};
use spin_serde::{DependencyPackageName, KebabId};
use std::path::{Path, PathBuf};

/// Parsed representation of the user-supplied source string.
#[derive(Clone, Debug)]
pub(super) enum ParsedSource {
    /// A local filesystem path to a Wasm component.
    Local(PathBuf),
    /// An HTTP(S) URL pointing to a Wasm component.
    Http(String),
    /// A registry package reference with an optional version constraint.
    Registry { package: DependencyPackageName },
    /// A reference to a component already defined in the manifest, by id.
    Component(KebabId),
}

impl std::str::FromStr for ParsedSource {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.starts_with("http://") || s.starts_with("https://") {
            Ok(ParsedSource::Http(s.to_string()))
        } else if s.contains('/') || s.contains('\\') || s.ends_with(".wasm") {
            Ok(ParsedSource::Local(PathBuf::from(s)))
        } else if s.contains(':') {
            // A package reference is namespaced, e.g. `my:package@1.0.0`.
            let package: DependencyPackageName = s
                .parse()
                .with_context(|| format!("failed to parse '{s}' as a dependency package name"))?;
            Ok(ParsedSource::Registry { package })
        } else {
            // A bare token (no scheme, path, or namespace separator) is treated as
            // a reference to a component defined in the manifest. Its existence
            // is checked when the source is resolved.
            let id = KebabId::try_from(s.to_string())
                .map_err(|e| anyhow!("'{s}' is not a valid component id: {e}"))?;
            Ok(ParsedSource::Component(id))
        }
    }
}

/// Where the dependency was resolved from, in the form recorded in the manifest.
pub(super) enum ResolvedSource {
    Local {
        path: PathBuf,
    },
    Http {
        url: String,
        digest: String,
    },
    Registry {
        version: String,
        registry: Option<String>,
        package: String,
    },
    Component {
        id: KebabId,
    },
}

impl ParsedSource {
    /// Check that `--digest` and `--registry` were given if and only if the source
    /// kind uses them, so bad invocations fail before any other work is done.
    pub(super) fn validate_options(
        &self,
        digest: Option<&str>,
        registry: Option<&str>,
    ) -> Result<()> {
        match self {
            ParsedSource::Http(_) if digest.is_none() => {
                bail!("HTTP sources require a SHA-256 digest; pass one with `--digest`")
            }
            ParsedSource::Http(_) => {}
            _ if digest.is_some() => {
                bail!("`--digest` only applies to HTTP sources")
            }
            _ => {}
        }
        if registry.is_some() && !matches!(self, ParsedSource::Registry { .. }) {
            bail!("`--registry` only applies to registry package sources");
        }
        Ok(())
    }

    /// Resolve the source to Wasm bytes plus the metadata needed to record it.
    ///
    /// `digest` and `registry` must already have passed [`Self::validate_options`].
    pub(super) async fn resolve(
        &self,
        digest: Option<&str>,
        registry: Option<&str>,
        app_root: &Path,
        manifest: &AppManifest,
    ) -> Result<(Vec<u8>, ResolvedSource)> {
        let loader = spin_loader::WasmLoader::new(app_root.to_owned(), None, None).await?;
        match self {
            ParsedSource::Local(path) => resolve_local(path, app_root).await,
            ParsedSource::Http(url) => {
                let digest = digest.context("HTTP sources require a digest")?;
                resolve_http(url, digest, &loader).await
            }
            ParsedSource::Registry { package } => {
                resolve_registry(package, registry, &loader).await
            }
            ParsedSource::Component(id) => resolve_component(id, manifest, &loader).await,
        }
    }
}

async fn resolve_local(path: &Path, app_root: &Path) -> Result<(Vec<u8>, ResolvedSource)> {
    // Resolve relative paths from the CWD (where the user typed the command),
    // not from app_root (where the manifest lives).
    let resolved = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .context("Failed to get current directory")?
            .join(path)
    };
    if !resolved.exists() {
        bail!("Dependency not found: {}", resolved.display());
    }
    // Store the path relative to app_root for the manifest, allowing `../`
    // segments for dependencies that live outside the app root.
    let canonical = resolved.canonicalize().unwrap_or_else(|_| resolved.clone());
    let base = app_root
        .canonicalize()
        .unwrap_or_else(|_| app_root.to_path_buf());
    let rel_path = pathdiff::diff_paths(&canonical, &base).unwrap_or(canonical);

    let bytes = tokio::fs::read(&resolved)
        .await
        .with_context(|| format!("Failed to read dependency at {}", resolved.display()))?;

    Ok((bytes, ResolvedSource::Local { path: rel_path }))
}

async fn resolve_http(
    url: &str,
    digest: &str,
    loader: &spin_loader::WasmLoader,
) -> Result<(Vec<u8>, ResolvedSource)> {
    let digest = format!("sha256:{digest}");
    let wasm_path = loader
        .load_http_source(url, &digest)
        .await
        .with_context(|| format!("Failed to download {url}"))?;
    let bytes = read_wasm(&wasm_path).await?;
    Ok((
        bytes,
        ResolvedSource::Http {
            url: url.to_string(),
            digest,
        },
    ))
}

async fn resolve_registry(
    package: &DependencyPackageName,
    registry: Option<&str>,
    loader: &spin_loader::WasmLoader,
) -> Result<(Vec<u8>, ResolvedSource)> {
    let version_req = match &package.version {
        Some(v) => semver::VersionReq::parse(&format!("={v}"))?,
        None => semver::VersionReq::STAR,
    };
    let registry_ref = registry
        .map(|r| {
            r.parse::<wasm_pkg_client::Registry>()
                .with_context(|| format!("'{r}' is not a valid registry"))
        })
        .transpose()?;

    let wasm_path = loader
        .load_registry_source(registry_ref.as_ref(), &package.package, &version_req)
        .await
        .context("Failed to load dependency from registry")?;
    let bytes = read_wasm(&wasm_path).await?;

    Ok((
        bytes,
        ResolvedSource::Registry {
            version: version_req.to_string(),
            registry: registry.map(str::to_string),
            package: package.package.to_string(),
        },
    ))
}

async fn resolve_component(
    id: &KebabId,
    manifest: &AppManifest,
    loader: &spin_loader::WasmLoader,
) -> Result<(Vec<u8>, ResolvedSource)> {
    let component = manifest.components.get(id).with_context(|| {
        format!("No component '{id}' found in the manifest to use as a dependency")
    })?;
    let wasm_path = loader
        .load_component_source(id.as_ref(), &component.source)
        .await
        .with_context(|| format!("Failed to load component '{id}'"))?;
    let bytes = read_wasm(&wasm_path).await?;
    Ok((bytes, ResolvedSource::Component { id: id.clone() }))
}

async fn read_wasm(path: &Path) -> Result<Vec<u8>> {
    tokio::fs::read(path)
        .await
        .with_context(|| format!("Failed to read dependency at {}", path.display()))
}

impl ResolvedSource {
    /// A short label for the source (for confirmation messages).
    pub(super) fn label(&self) -> String {
        match self {
            ResolvedSource::Local { path } => path.display().to_string(),
            ResolvedSource::Http { url, .. } => url.clone(),
            ResolvedSource::Registry {
                package, version, ..
            } => format!("{package}@{version}"),
            ResolvedSource::Component { id } => id.to_string(),
        }
    }

    /// The `ComponentDependency` manifest entry for this source.
    pub(super) fn to_component_dependency(
        &self,
        inherit_configuration: Option<InheritConfiguration>,
        capabilities: Option<spin_manifest::schema::v2::DependencyCapabilities>,
    ) -> ComponentDependency {
        match self {
            ResolvedSource::Local { path } => ComponentDependency::Local {
                path: path.clone(),
                export: None,
                inherit_configuration,
                capabilities,
            },
            ResolvedSource::Http { url, digest } => ComponentDependency::HTTP {
                url: url.clone(),
                digest: digest.clone(),
                export: None,
                inherit_configuration,
                capabilities,
            },
            ResolvedSource::Registry {
                version,
                registry,
                package,
            } => ComponentDependency::Package {
                version: version.clone(),
                registry: registry.clone(),
                package: Some(package.clone()),
                export: None,
                inherit_configuration,
                capabilities,
            },
            ResolvedSource::Component { id } => ComponentDependency::AppComponent {
                component: id.clone(),
                export: None,
                inherit_configuration,
                capabilities,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_sources() {
        assert!(matches!(
            "https://example.com/c.wasm"
                .parse::<ParsedSource>()
                .unwrap(),
            ParsedSource::Http(_)
        ));
        assert!(matches!(
            "http://example.com/c.wasm".parse::<ParsedSource>().unwrap(),
            ParsedSource::Http(_)
        ));
    }

    #[test]
    fn parses_local_sources() {
        assert!(matches!(
            "./c.wasm".parse::<ParsedSource>().unwrap(),
            ParsedSource::Local(_)
        ));
        assert!(matches!(
            "path/to/c.wasm".parse::<ParsedSource>().unwrap(),
            ParsedSource::Local(_)
        ));
        assert!(matches!(
            "component.wasm".parse::<ParsedSource>().unwrap(),
            ParsedSource::Local(_)
        ));
    }

    #[test]
    fn parses_registry_sources() {
        let ParsedSource::Registry { package } =
            "my:package@1.0.0".parse::<ParsedSource>().unwrap()
        else {
            panic!("expected registry source");
        };
        assert_eq!(package.package.to_string(), "my:package");
        assert_eq!(package.version.map(|v| v.to_string()), Some("1.0.0".into()));
    }

    #[test]
    fn registry_source_without_version() {
        let ParsedSource::Registry { package } = "my:package".parse::<ParsedSource>().unwrap()
        else {
            panic!("expected registry source");
        };
        assert_eq!(package.package.to_string(), "my:package");
        assert!(package.version.is_none());
    }

    #[test]
    fn parses_component_reference() {
        let ParsedSource::Component(id) = "ensure-admin".parse::<ParsedSource>().unwrap() else {
            panic!("expected a component reference");
        };
        assert_eq!(id.as_ref(), "ensure-admin");
    }

    #[test]
    fn invalid_component_reference_errors() {
        assert!("not_kebab".parse::<ParsedSource>().is_err());
    }

    #[test]
    fn invalid_registry_source_errors() {
        // A namespaced-looking token that is not a valid package reference should
        // error rather than be misclassified.
        assert!("my:@@bad".parse::<ParsedSource>().is_err());
    }

    #[test]
    fn options_must_match_source_kind() {
        let http: ParsedSource = "https://example.com/c.wasm".parse().unwrap();
        let local: ParsedSource = "./c.wasm".parse().unwrap();
        let registry: ParsedSource = "my:package".parse().unwrap();

        assert!(http.validate_options(Some("abc"), None).is_ok());
        assert!(http.validate_options(None, None).is_err());
        assert!(http.validate_options(Some("abc"), Some("reg")).is_err());

        assert!(local.validate_options(None, None).is_ok());
        assert!(local.validate_options(Some("abc"), None).is_err());
        assert!(local.validate_options(None, Some("reg")).is_err());

        assert!(registry.validate_options(None, Some("reg")).is_ok());
        assert!(registry.validate_options(None, None).is_ok());
        assert!(registry.validate_options(Some("abc"), None).is_err());
    }
}
