use crate::opts::*;
use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use spin_common::paths::parent_dir;
use spin_manifest::schema::v2::{AppManifest, ComponentDependency, InheritConfiguration};
use spin_serde::{DependencyName, DependencyPackageName, KebabId};
use std::path::PathBuf;

/// Commands for managing component dependencies.
#[derive(Subcommand, Debug)]
pub enum DepsCommands {
    /// Add a component dependency to a component in the application.
    Add(AddCommand),
}

impl DepsCommands {
    pub async fn run(self) -> Result<()> {
        match self {
            DepsCommands::Add(cmd) => cmd.run().await,
        }
    }
}

#[derive(Parser, Debug)]
pub struct AddCommand {
    /// The dependency source: a file path, HTTP URL, or registry package reference.
    ///
    /// Examples:
    ///   ./path/to/component.wasm
    ///   https://example.com/component.wasm
    ///   my:package@1.0.0
    source: ParsedSource,

    /// Sha256 digest that will be used to verify HTTP downloads. Required for HTTP sources, ignored otherwise.
    #[clap(short, long)]
    digest: Option<String>,

    /// Registry to override the default with. Ignored in the cases of local or HTTP sources.
    #[clap(short, long)]
    registry: Option<String>,

    /// The component to add the dependency to. If omitted and the app has
    /// exactly one component, it is selected automatically; otherwise you
    /// will be prompted.
    #[clap(long = "to")]
    component_id: Option<String>,

    /// Path to the application manifest (spin.toml).
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
    )]
    app_source: Option<PathBuf>,

    /// The export(s) to use from the dependency, as a DependencyName
    /// (e.g., `foo:bar/baz@0.1.0` or `my-export`). Can also be a package
    /// selector like `foo:bar` or `foo:bar@0.1.0` to use all exports from
    /// that package. If omitted and the component has multiple exports,
    /// you will be prompted.
    #[clap(long = "export")]
    export: Option<String>,

    /// Capabilities to inherit from the parent component.
    /// Use `--inherit true` to inherit all, `--inherit false` for none,
    /// or comma-separate individual capabilities (e.g. `--inherit allowed_outbound_hosts,ai_models`).
    /// If omitted and the dependency requires capabilities, you will be prompted.
    #[clap(long = "inherit")]
    inherit: Option<CliInheritConfiguration>,
}

/// CLI representation of `InheritConfiguration`, mirroring the manifest type.
#[derive(Clone, Debug)]
pub enum CliInheritConfiguration {
    /// Inherit all configurations from the parent.
    All,
    /// Inherit none.
    None,
    /// Inherit only the specified capabilities.
    Some(Vec<String>),
}

impl std::str::FromStr for CliInheritConfiguration {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "true" | "all" => Ok(Self::All),
            "false" | "none" => Ok(Self::None),
            other => {
                let caps = other.split(',').map(|s| s.trim().to_string()).collect();
                Ok(Self::Some(caps))
            }
        }
    }
}

/// Parsed representation of the user-supplied source string.
#[derive(Clone, Debug)]
enum ParsedSource {
    /// A local filesystem path to a Wasm component.
    Local(PathBuf),
    /// An HTTP(S) URL pointing to a Wasm component.
    Http(String),
    /// A registry package reference with an optional version constraint.
    Registry { package: DependencyPackageName },
}

impl std::str::FromStr for ParsedSource {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s.starts_with("http://") || s.starts_with("https://") {
            Ok(ParsedSource::Http(s.to_string()))
        } else if s.contains('/') || s.contains('\\') || s.ends_with(".wasm") {
            Ok(ParsedSource::Local(PathBuf::from(s)))
        } else {
            // Treat as registry package reference
            let package: DependencyPackageName = s
                .parse()
                .with_context(|| format!("failed to parse '{s}' as a dependency package name"))?;
            Ok(ParsedSource::Registry { package })
        }
    }
}

/// Resolved source information needed to build the ComponentDependency value.
enum ResolvedSource {
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
        package: Option<String>,
    },
}

impl AddCommand {
    pub async fn run(self) -> Result<()> {
        // Locate and parse the manifest
        let (manifest_file, _) =
            spin_common::paths::find_manifest_file_path(self.app_source.as_ref())?;
        let manifest_file = manifest_file.canonicalize().with_context(|| {
            format!(
                "Failed to canonicalize manifest path: {}",
                manifest_file.display()
            )
        })?;
        let app_root = parent_dir(&manifest_file)?;

        let manifest = spin_manifest::manifest_from_file(&manifest_file)?;

        // Select the target component
        let component_id = self.resolve_component_id(&manifest)?;

        // Resolve to a wasm bytes
        let (wasm_bytes, dep_source) = self.resolve_source(&app_root).await?;

        // Select the export
        let selected_export = self.resolve_exported(&wasm_bytes)?;

        // Determine capabilities needed by the dependency
        let inherit_config = self.resolve_inherit_configuration(&wasm_bytes)?;

        // Build and write the dependency
        let dep_value = build_component_dependency(&dep_source, inherit_config.clone())?;
        let dep_name: DependencyName = selected_export.parse().with_context(|| {
            format!("Failed to parse selected export '{selected_export}' as a DependencyName")
        })?;
        write_dependency_to_manifest(&manifest_file, &component_id, &dep_name, &dep_value)?;

        // Regenerate spin-dependencies.wit
        let manifest = spin_manifest::manifest_from_file(&manifest_file)?;
        let component_kebab: KebabId = component_id
            .clone()
            .try_into()
            .map_err(|e| anyhow!("{e}"))?;
        let component = manifest
            .components
            .get(&component_kebab)
            .with_context(|| format!("Component '{component_id}' not found after writing"))?;

        let component_dir = match component.build.as_ref().and_then(|b| b.workdir.as_ref()) {
            None => app_root.to_owned(),
            Some(d) => app_root.join(d),
        };
        let dest_file = component_dir.join("spin-dependencies.wit");

        spin_dependency_wit::extract_wits_into(
            component.dependencies.inner.iter(),
            &app_root,
            &dest_file,
        )
        .await
        .context("Failed to regenerate spin-dependencies.wit")?;

        println!("Added {selected_export} to component '{component_id}'");

        let capabilities = match inherit_config {
            Some(InheritConfiguration::Some(names)) => Some(names),
            Some(InheritConfiguration::All(true)) => Some(
                spin_capabilities::CAPABILITY_SETS
                    .iter()
                    .map(|(name, _)| name.to_string())
                    .collect(),
            ),
            _ => None,
        };

        if let Some(capabilities) = capabilities {
            println!();
            println!(
                "NOTE: This dependency requires the following capabilities: {}",
                capabilities.join(", ")
            );
            println!("You may need to add configuration for these capabilities to your component.");
        }

        Ok(())
    }

    /// Determine which component to add the dependency to.
    fn resolve_component_id(&self, manifest: &AppManifest) -> Result<String> {
        if let Some(id) = &self.component_id {
            // Validate it exists
            let kebab: KebabId = id
                .parse::<String>()
                .unwrap()
                .try_into()
                .map_err(|e| anyhow!("{e}"))?;
            if !manifest.components.contains_key(&kebab) {
                bail!(
                    "Component '{}' not found in manifest. Available components: {}",
                    id,
                    manifest
                        .components
                        .keys()
                        .map(|k| k.as_ref())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            return Ok(id.clone());
        }

        let component_ids: Vec<String> = manifest
            .components
            .keys()
            .map(|k| k.as_ref().to_string())
            .collect();

        if component_ids.is_empty() {
            bail!("No components found in the manifest");
        }

        if component_ids.len() == 1 {
            return Ok(component_ids.into_iter().next().unwrap());
        }

        let selection = dialoguer::Select::new()
            .with_prompt("Which component should the dependency be added to?")
            .items(&component_ids)
            .interact()
            .context("Failed to select component")?;

        Ok(component_ids[selection].clone())
    }

    /// Resolve the parsed source to a Wasm binary on disk and source metadata.
    async fn resolve_source(
        &self,
        app_root: &std::path::Path,
    ) -> Result<(Vec<u8>, ResolvedSource)> {
        match &self.source {
            ParsedSource::Local(path) => {
                // Resolve relative paths from CWD (where the user typed the command),
                // not from app_root (where the manifest lives).
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    std::env::current_dir()
                        .context("Failed to get current directory")?
                        .join(path)
                };
                if !resolved.exists() {
                    bail!("Dependency not found: {}", resolved.display());
                }
                // Store the path relative to app_root for the manifest
                let rel_path = resolved
                    .canonicalize()
                    .unwrap_or(resolved.clone())
                    .strip_prefix(app_root.canonicalize().unwrap_or(app_root.to_path_buf()))
                    .map(|p| p.to_path_buf())
                    .unwrap_or(resolved.clone());

                let bytes = tokio::fs::read(&resolved).await.with_context(|| {
                    format!("Failed to read dependency at {}", resolved.display())
                })?;

                Ok((bytes, ResolvedSource::Local { path: rel_path }))
            }
            ParsedSource::Http(url) => {
                let cache = spin_loader::cache::Cache::new(None).await?;
                let digest = self
                    .digest
                    .clone()
                    .map(|digest| format!("sha256:{digest}"))
                    .ok_or_else(|| {
                        anyhow::anyhow!("Digest needs to be specified for HTTP sources.")
                    })?;

                if let Ok(path) = cache.wasm_file(&digest) {
                    let bytes = tokio::fs::read(&path).await.with_context(|| {
                        format!("Failed to read dependency at {}", path.display())
                    })?;

                    return Ok((
                        bytes,
                        ResolvedSource::Http {
                            url: url.clone(),
                            digest,
                        },
                    ));
                }

                let response = reqwest::get(url)
                    .await
                    .with_context(|| format!("Failed to download {url}"))?;
                if !response.status().is_success() {}
                let bytes = response
                    .bytes()
                    .await
                    .with_context(|| format!("Failed to read response body from {url}"))?;

                let actual_digest = {
                    use sha2::Digest;
                    let hash = sha2::Sha256::digest(&bytes);
                    format!("sha256:{hash:x}")
                };

                anyhow::ensure!(
                    actual_digest == digest,
                    "invalid content digest; expected {digest}, downloaded {actual_digest}"
                );

                let dest = cache.wasm_path(&digest);
                tokio::fs::write(dest, &bytes).await?;

                Ok((
                    bytes.to_vec(),
                    ResolvedSource::Http {
                        url: url.clone(),
                        digest,
                    },
                ))
            }
            ParsedSource::Registry { package } => {
                let loader = spin_loader::WasmLoader::new(app_root.to_owned(), None, None).await?;

                // Build a temporary ComponentDependency::Package to use load_dependency_content
                let version_str = package
                    .version
                    .as_ref()
                    .map(|v| format!("={v}"))
                    .unwrap_or_else(|| "*".to_string());

                let dep_name = DependencyName::Package(package.clone());
                let temp_dep = ComponentDependency::Package {
                    version: version_str.clone(),
                    registry: self.registry.clone(),
                    package: Some(package.package.to_string()),
                    export: None,
                    inherit_configuration: None,
                };

                let (wasm_path, _export) = loader
                    .load_dependency_content(&dep_name, &temp_dep)
                    .await
                    .context("Failed to load dependency from registry")?;

                let bytes = tokio::fs::read(&wasm_path).await.with_context(|| {
                    format!("Failed to read dependency at {}", wasm_path.display())
                })?;

                Ok((
                    bytes,
                    ResolvedSource::Registry {
                        version: version_str,
                        registry: None,
                        package: Some(package.package.to_string()),
                    },
                ))
            }
        }
    }

    /// Determine which export(s) to use from the dependency component.
    ///
    /// The `--export` flag accepts:
    /// - A specific interface: `foo:bar/baz@0.1.0` → selects that one export
    /// - A package selector: `foo:bar` or `foo:bar@0.1.0` → selects all exports in that package
    /// - A plain name: `my-export` → selects that named export
    ///
    /// Without `--export`, prompts interactively (auto-selects if only one).
    fn resolve_exported(&self, wasm_bytes: &[u8]) -> Result<String> {
        let exports = spin_dependency_wit::list_exports(wasm_bytes)
            .context("Failed to enumerate exports from the Wasm component")?;

        if exports.is_empty() {
            bail!("The Wasm component has no exports to use as a dependency");
        }

        if let Some(export) = &self.export {
            let parsed: DependencyName = export
                .parse()
                .with_context(|| format!("'{export}' is not a valid dependency name"))?;

            match &parsed {
                DependencyName::Package(pkg) if pkg.interface.is_none() => {
                    // Package-level selector: match all exports belonging to this package
                    let matched: Vec<String> = exports
                        .iter()
                        .filter(|e| {
                            let Ok(export_pkg) = e.parse::<DependencyPackageName>() else {
                                return false;
                            };
                            if export_pkg.package != pkg.package {
                                return false;
                            }
                            // If the selector specifies a version, the export must match it
                            match (&pkg.version, &export_pkg.version) {
                                (Some(selector_ver), Some(export_ver)) => {
                                    selector_ver == export_ver
                                }
                                (Some(_), None) => false,
                                _ => true,
                            }
                        })
                        .cloned()
                        .collect();

                    if matched.is_empty() {
                        bail!(
                            "No exports match package '{}'. Available exports: {}",
                            export,
                            exports.join(", ")
                        );
                    }
                    Ok(export.clone())
                }
                _ => {
                    // Specific interface or plain name — select exactly that export
                    if !exports.contains(&export.to_string()) {
                        bail!(
                            "Export '{}' not found. Available exports: {}",
                            export,
                            exports.join(", ")
                        );
                    }
                    Ok(export.clone())
                }
            }
        } else if exports.len() == 1 {
            Ok(exports[0].clone())
        } else {
            // Group exports by package (plain-named exports go under a "" key)
            let mut package_exports: indexmap::IndexMap<String, Vec<String>> =
                indexmap::IndexMap::new();
            for export in &exports {
                let pkg_key = export
                    .parse::<DependencyPackageName>()
                    .ok()
                    .map(|p| {
                        let mut key = p.package.to_string();
                        if let Some(v) = &p.version {
                            key.push_str(&format!("@{v}"));
                        }
                        key
                    })
                    .unwrap_or_default();
                package_exports
                    .entry(pkg_key)
                    .or_default()
                    .push(export.clone());
            }

            // Select a package (auto-select if only one)
            let package_keys: Vec<&String> = package_exports.keys().collect();
            let selected_pkg = if package_keys.len() == 1 {
                package_keys[0].clone()
            } else {
                let labels: Vec<&str> = package_keys
                    .iter()
                    .map(|k| {
                        if k.is_empty() {
                            "<unnamed exports>"
                        } else {
                            k.as_str()
                        }
                    })
                    .collect();
                let idx = dialoguer::Select::new()
                    .with_prompt("Which package should be used?")
                    .items(&labels)
                    .interact()
                    .context("Failed to select package")?;
                package_keys[idx].clone()
            };

            let pkg_interfaces = &package_exports[&selected_pkg];

            // Select all or a specific interface
            if pkg_interfaces.len() == 1 {
                Ok(pkg_interfaces[0].clone())
            } else {
                let all_label = format!(
                    "All from {}",
                    if selected_pkg.is_empty() {
                        "<unnamed>"
                    } else {
                        &selected_pkg
                    }
                );
                let mut items = vec![all_label.as_str()];
                items.extend(pkg_interfaces.iter().map(|s| s.as_str()));

                let selection = dialoguer::Select::new()
                    .with_prompt("Which export should be used?")
                    .items(&items)
                    .interact()
                    .context("Failed to select export")?;

                if selection == 0 {
                    // "All" was selected — return the package-level selector
                    if selected_pkg.is_empty() {
                        bail!("Cannot select all from unnamed exports");
                    }
                    Ok(selected_pkg)
                } else {
                    Ok(items[selection].to_string())
                }
            }
        }
    }

    /// Determine the inherit_configuration value.
    fn resolve_inherit_configuration(
        &self,
        wasm_bytes: &[u8],
    ) -> Result<Option<InheritConfiguration>> {
        let capabilities = spin_capabilities::InheritConfiguration::collect(wasm_bytes)
            .context("Failed to collect capability requirements from the dependency")?;

        // If no capabilities are required, nothing to inherit
        let Some(spin_capabilities::InheritConfiguration::Some(required_caps)) = capabilities
        else {
            return Ok(None);
        };

        if required_caps.is_empty() {
            return Ok(None);
        }

        // If --inherit was provided, use the typed value directly
        if let Some(inherit) = &self.inherit {
            match inherit {
                CliInheritConfiguration::All => {
                    return Ok(Some(InheritConfiguration::All(true)));
                }
                CliInheritConfiguration::None => {
                    return Ok(None);
                }
                CliInheritConfiguration::Some(caps) => {
                    // Validate against the required capabilities
                    for cap in caps {
                        if !required_caps.contains(cap) {
                            bail!(
                                "Capability '{}' is not required by this dependency. Required: {}",
                                cap,
                                required_caps.join(", ")
                            );
                        }
                    }
                    return Ok(Some(InheritConfiguration::Some(caps.clone())));
                }
            }
        }

        // Interactive: prompt with Select for all or specific capabilities
        println!(
            "This dependency requires the following capabilities: {}",
            required_caps.join(", ")
        );

        let all_label = "All".to_string();
        let mut items = vec![all_label.as_str()];
        items.extend(required_caps.iter().map(|s| s.as_str()));

        let selections = dialoguer::MultiSelect::new()
            .with_prompt("Select capabilities to inherit from the parent component")
            .items(&items)
            .interact()
            .context("Failed to select capabilities")?;

        if selections.is_empty() {
            return Ok(None);
        }

        if selections.contains(&0) {
            return Ok(Some(InheritConfiguration::All(true)));
        }

        let selected: Vec<String> = selections
            .into_iter()
            .map(|i| items[i].to_string())
            .collect();

        Ok(Some(InheritConfiguration::Some(selected)))
    }
}

/// Build the ComponentDependency TOML value from the resolved source.
fn build_component_dependency(
    source: &ResolvedSource,
    inherit_config: Option<InheritConfiguration>,
) -> Result<ComponentDependency> {
    match source {
        ResolvedSource::Local { path } => Ok(ComponentDependency::Local {
            path: path.clone(),
            export: None,
            inherit_configuration: inherit_config,
        }),
        ResolvedSource::Http { url, digest } => Ok(ComponentDependency::HTTP {
            url: url.clone(),
            digest: digest.clone(),
            export: None,
            inherit_configuration: inherit_config,
        }),
        ResolvedSource::Registry {
            version,
            registry,
            package,
        } => Ok(ComponentDependency::Package {
            version: version.clone(),
            registry: registry.clone(),
            package: package.clone(),
            export: None,
            inherit_configuration: inherit_config,
        }),
    }
}

/// Write the dependency into the spin.toml manifest using toml_edit.
fn write_dependency_to_manifest(
    manifest_file: &std::path::Path,
    component_id: &str,
    dep_name: &DependencyName,
    dep_value: &ComponentDependency,
) -> Result<()> {
    use toml_edit::{DocumentMut, Item, Table};

    let manifest_text =
        std::fs::read_to_string(manifest_file).context("Failed to read manifest file")?;
    let mut doc: DocumentMut = manifest_text
        .parse()
        .context("Failed to parse manifest as TOML")?;

    // Navigate to [component.<component_id>]
    let component_table = doc
        .get_mut("component")
        .and_then(|c| c.as_table_like_mut())
        .with_context(|| "No [component] table in manifest")?;

    let component = component_table
        .get_mut(component_id)
        .and_then(|c| c.as_table_like_mut())
        .with_context(|| format!("Component '{component_id}' not found in manifest"))?;

    // Ensure [component.<id>.dependencies] exists
    if component.get("dependencies").is_none() {
        component.insert("dependencies", Item::Table(Table::new()));
    }
    let deps_table = component
        .get_mut("dependencies")
        .and_then(|d| d.as_table_like_mut())
        .context("Failed to access dependencies table")?;

    let dep_key = dep_name.to_string();
    if deps_table.contains_key(&dep_key) {
        bail!(
            "Dependency '{}' already exists in component '{}'",
            dep_key,
            component_id
        );
    }

    // Serialize the dependency value
    let dep_toml_value = serialize_component_dependency(dep_value)?;
    deps_table.insert(&dep_key, dep_toml_value);

    std::fs::write(manifest_file, doc.to_string()).context("Failed to write manifest file")?;

    Ok(())
}

/// Serialize a ComponentDependency into a toml_edit::Item.
fn serialize_component_dependency(dep: &ComponentDependency) -> Result<toml_edit::Item> {
    match dep {
        ComponentDependency::Version(version) => Ok(toml_edit::value(version.as_str())),
        ComponentDependency::Local {
            path,
            export,
            inherit_configuration,
        } => {
            let mut table = toml_edit::InlineTable::new();
            table.insert(
                "path",
                toml_edit::Value::from(path.to_string_lossy().as_ref()),
            );
            if let Some(export) = export {
                table.insert("export", toml_edit::Value::from(export.as_str()));
            }
            insert_inherit_configuration(&mut table, inherit_configuration);
            Ok(toml_edit::Item::Value(toml_edit::Value::InlineTable(table)))
        }
        ComponentDependency::HTTP {
            url,
            digest,
            export,
            inherit_configuration,
        } => {
            let mut table = toml_edit::InlineTable::new();
            table.insert("url", toml_edit::Value::from(url.as_str()));
            table.insert("digest", toml_edit::Value::from(digest.as_str()));
            if let Some(export) = export {
                table.insert("export", toml_edit::Value::from(export.as_str()));
            }
            insert_inherit_configuration(&mut table, inherit_configuration);
            Ok(toml_edit::Item::Value(toml_edit::Value::InlineTable(table)))
        }
        ComponentDependency::Package {
            version,
            registry,
            package,
            export,
            inherit_configuration,
        } => {
            let mut table = toml_edit::InlineTable::new();
            table.insert("version", toml_edit::Value::from(version.as_str()));
            if let Some(registry) = registry {
                table.insert("registry", toml_edit::Value::from(registry.as_str()));
            }
            if let Some(package) = package {
                table.insert("package", toml_edit::Value::from(package.as_str()));
            }
            if let Some(export) = export {
                table.insert("export", toml_edit::Value::from(export.as_str()));
            }
            insert_inherit_configuration(&mut table, inherit_configuration);
            Ok(toml_edit::Item::Value(toml_edit::Value::InlineTable(table)))
        }
        ComponentDependency::AppComponent {
            component,
            export,
            inherit_configuration,
        } => {
            let mut table = toml_edit::InlineTable::new();
            table.insert("component", toml_edit::Value::from(component.as_ref()));
            if let Some(export) = export {
                table.insert("export", toml_edit::Value::from(export.as_str()));
            }
            insert_inherit_configuration(&mut table, inherit_configuration);
            Ok(toml_edit::Item::Value(toml_edit::Value::InlineTable(table)))
        }
    }
}

fn insert_inherit_configuration(
    table: &mut toml_edit::InlineTable,
    config: &Option<InheritConfiguration>,
) {
    match config {
        None => {}
        Some(InheritConfiguration::All(val)) => {
            table.insert("inherit_configuration", toml_edit::Value::from(*val));
        }
        Some(InheritConfiguration::Some(keys)) => {
            let mut arr = toml_edit::Array::new();
            for key in keys {
                arr.push(key.as_str());
            }
            table.insert("inherit_configuration", toml_edit::Value::Array(arr));
        }
    }
}
