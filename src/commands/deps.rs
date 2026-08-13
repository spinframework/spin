use crate::opts::*;
use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use spin_common::paths::parent_dir;
use spin_manifest::schema::v2::{
    AppManifest, ComponentDependency, ComponentSpec, InheritConfiguration, Trigger,
    TriggerDependency,
};
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
    /// The dependency source: a local file path, an HTTP(S) URL, or a registry
    /// package reference.
    ///
    /// Examples:
    ///   ./path/to/component.wasm
    ///   https://example.com/component.wasm
    ///   my:package@1.0.0
    source: ParsedSource,

    /// SHA-256 digest used to verify HTTP downloads. Required for HTTP sources,
    /// ignored otherwise.
    #[clap(short = 'd', long = "digest")]
    digest: Option<String>,

    /// Registry to override the default with. Only applies to registry sources.
    #[clap(short = 'r', long = "registry")]
    registry: Option<String>,

    /// Path to the application manifest (spin.toml). Defaults to the current
    /// directory.
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
    )]
    app_source: Option<PathBuf>,
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
    /// A reference to a component already defined in the manifest, by id.
    Component(String),
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
            // a reference to a component defined in the manifest. It is validated
            // against the manifest when the source is resolved.
            Ok(ParsedSource::Component(s.to_string()))
        }
    }
}

/// Resolved source information needed to build the `ComponentDependency` value.
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
    Component {
        id: String,
    },
}

impl AddCommand {
    pub async fn run(self) -> Result<()> {
        // Locate and parse the manifest.
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

        // Resolve the source to Wasm bytes plus the metadata needed to record it.
        let (wasm_bytes, dep_source) = self.resolve_source(&app_root, &manifest).await?;

        // A component that both imports and exports wasi:http/handler is HTTP
        // middleware, which is attached to a trigger rather than a component.
        if spin_dependency_wit::is_http_middleware(&wasm_bytes)
            .context("Failed to inspect the component's interfaces")?
        {
            self.add_middleware(&manifest_file, &manifest, &wasm_bytes, dep_source)
                .await
        } else {
            self.add_component_dependency(
                &manifest_file,
                &app_root,
                &manifest,
                &wasm_bytes,
                dep_source,
            )
            .await
        }
    }

    /// Add the resolved source as a component dependency to a selected component.
    async fn add_component_dependency(
        &self,
        manifest_file: &std::path::Path,
        app_root: &std::path::Path,
        manifest: &AppManifest,
        wasm_bytes: &[u8],
        dep_source: ResolvedSource,
    ) -> Result<()> {
        // Select the target component.
        let component_id = self.resolve_component_id(manifest)?;

        // Select the interface to import.
        let selected = resolve_interface(wasm_bytes)?;

        // Determine capability inheritance.
        let (inherit_config, required_caps) =
            resolve_inherit_configuration(manifest, &component_id, wasm_bytes)?;

        // Build and write the dependency into the manifest.
        let dep_name: DependencyName = selected.parse().with_context(|| {
            format!("Failed to parse selected interface '{selected}' as a dependency name")
        })?;
        let dep_value = build_component_dependency(&dep_source, inherit_config.clone())?;
        write_dependency_to_manifest(manifest_file, &component_id, &dep_name, &dep_value)?;

        // Regenerate spin-dependencies.wit for the component.
        let manifest = spin_manifest::manifest_from_file(manifest_file)?;
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
            app_root,
            &dest_file,
        )
        .await
        .context("Failed to regenerate spin-dependencies.wit")?;

        // Report success and any capability guidance.
        println!("Added {selected} to component '{component_id}'");
        println!("Run `spin build` to generate language bindings for the new dependency.");
        print_capability_guidance(&component_id, &required_caps, inherit_config.as_ref());

        Ok(())
    }

    /// Attach the resolved source as HTTP middleware to a selected trigger.
    async fn add_middleware(
        &self,
        manifest_file: &std::path::Path,
        manifest: &AppManifest,
        wasm_bytes: &[u8],
        dep_source: ResolvedSource,
    ) -> Result<()> {
        println!("Detected HTTP middleware (imports and exports wasi:http/handler).");
        println!();

        // Enumerate HTTP triggers that have a route; private (route-less)
        // endpoints are not eligible for middleware.
        let http_triggers = manifest.triggers.get("http").cloned().unwrap_or_default();
        let routed: Vec<&Trigger> = http_triggers
            .iter()
            .filter(|t| trigger_route(t).is_some())
            .collect();

        if routed.is_empty() {
            bail!("The application has no routed HTTP triggers to attach middleware to.");
        }

        // Select the target trigger by route.
        let trigger = if routed.len() == 1 {
            routed[0]
        } else {
            let routes: Vec<&str> = routed.iter().filter_map(|t| trigger_route(t)).collect();
            let sel = dialoguer::Select::new()
                .with_prompt("Which HTTP route should the middleware be added to?")
                .items(&routes)
                .interact()
                .context("Failed to select route")?;
            routed[sel]
        };
        let route = trigger_route(trigger)
            .context("selected trigger has no route")?
            .to_string();

        // Existing middleware entries on the trigger (used for positioning).
        let existing_labels: Vec<String> = trigger
            .dependencies
            .get("middleware")
            .map(|d| d.0.iter().map(trigger_dependency_label).collect())
            .unwrap_or_default();

        // Choose the pipeline position (append when the pipeline is empty).
        let index = if existing_labels.is_empty() {
            0
        } else {
            let mut items = vec!["At the end (closest to the application component)".to_string()];
            for label in &existing_labels {
                items.push(format!("Before {label}"));
            }
            let sel = dialoguer::Select::new()
                .with_prompt("Where should this middleware run in the pipeline?")
                .items(&items)
                .default(0)
                .interact()
                .context("Failed to select pipeline position")?;
            if sel == 0 {
                existing_labels.len()
            } else {
                sel - 1
            }
        };

        // Capability inheritance (from the component the trigger routes to).
        let required_caps = collect_required_capabilities(wasm_bytes)?;
        let inherit_config = if required_caps.is_empty() {
            None
        } else {
            println!(
                "This middleware requires the following capabilities: {}",
                required_caps.join(", ")
            );
            let selections = dialoguer::MultiSelect::new()
                .with_prompt("Select capabilities to inherit from the trigger's component")
                .items(&required_caps)
                .interact()
                .context("Failed to select capabilities")?;
            if selections.is_empty() {
                None
            } else {
                let selected = selections
                    .into_iter()
                    .map(|i| required_caps[i].clone())
                    .collect();
                Some(InheritConfiguration::Some(selected))
            }
        };

        // Build and write the middleware entry.
        let entry = serialize_trigger_dependency(&dep_source, inherit_config.as_ref());
        write_middleware_to_manifest(manifest_file, &route, index, entry)?;

        println!(
            "Added middleware '{}' to the trigger for route '{route}'",
            source_label(&dep_source)
        );

        let component_id = match &trigger.component {
            Some(ComponentSpec::Reference(k)) => Some(k.as_ref().to_string()),
            _ => None,
        };
        print_middleware_capability_guidance(
            component_id.as_deref(),
            &required_caps,
            inherit_config.as_ref(),
        );

        Ok(())
    }

    /// Determine which component to add the dependency to (interactive).
    fn resolve_component_id(&self, manifest: &AppManifest) -> Result<String> {
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

    /// Resolve the source to Wasm bytes on disk and the metadata needed to record it.
    async fn resolve_source(
        &self,
        app_root: &std::path::Path,
        manifest: &AppManifest,
    ) -> Result<(Vec<u8>, ResolvedSource)> {
        match &self.source {
            ParsedSource::Local(path) => {
                // Resolve relative paths from the CWD (where the user typed the
                // command), not from app_root (where the manifest lives).
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
                // Store the path relative to app_root for the manifest.
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
                    .ok_or_else(|| anyhow!("A digest must be specified for HTTP sources."))?;

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
                if !response.status().is_success() {
                    bail!("Failed to download {}: HTTP {}", url, response.status());
                }
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
                        registry: self.registry.clone(),
                        package: Some(package.package.to_string()),
                    },
                ))
            }
            ParsedSource::Component(id) => {
                let kebab: KebabId = id
                    .clone()
                    .try_into()
                    .map_err(|e| anyhow!("'{id}' is not a valid component id: {e}"))?;
                let component = manifest.components.get(&kebab).with_context(|| {
                    format!("No component '{id}' found in the manifest to use as a dependency")
                })?;
                let loader = spin_loader::WasmLoader::new(app_root.to_owned(), None, None).await?;
                let wasm_path = loader
                    .load_component_source(id, &component.source)
                    .await
                    .with_context(|| format!("Failed to load component '{id}'"))?;
                let bytes = tokio::fs::read(&wasm_path).await.with_context(|| {
                    format!("Failed to read component '{id}' at {}", wasm_path.display())
                })?;
                Ok((bytes, ResolvedSource::Component { id: id.clone() }))
            }
        }
    }
}

/// Determine which interface to import from the dependency component (interactive).
///
/// Presents a single flat list of all interface exports. For each package that
/// exposes more than one interface, an "All from <package>" entry is included so
/// the whole package can be selected as a package-level dependency.
fn resolve_interface(wasm_bytes: &[u8]) -> Result<String> {
    let exports = spin_dependency_wit::list_exports(wasm_bytes)
        .context("Failed to enumerate exports from the Wasm component")?;

    if exports.is_empty() {
        bail!("The Wasm component has no exports to use as a dependency");
    }

    if exports.len() == 1 {
        return Ok(exports[0].clone());
    }

    // Group exports by package, preserving first-seen order. Plain-named exports
    // (which have no package) are grouped under `None`.
    let mut groups: Vec<(Option<String>, Vec<String>)> = Vec::new();
    for export in &exports {
        let pkg_key = export.parse::<DependencyPackageName>().ok().map(|p| {
            let mut key = p.package.to_string();
            if let Some(v) = &p.version {
                key.push_str(&format!("@{v}"));
            }
            key
        });
        match groups.iter_mut().find(|(k, _)| *k == pkg_key) {
            Some((_, v)) => v.push(export.clone()),
            None => groups.push((pkg_key, vec![export.clone()])),
        }
    }

    // Build a flat list of (label, value) items. `value` is the dependency-name
    // string recorded when that item is selected.
    let mut labels: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for (pkg_key, interfaces) in &groups {
        match pkg_key {
            Some(pkg) if interfaces.len() > 1 => {
                labels.push(format!("All from {pkg}"));
                values.push(pkg.clone());
                for itf in interfaces {
                    labels.push(itf.clone());
                    values.push(itf.clone());
                }
            }
            _ => {
                for itf in interfaces {
                    labels.push(itf.clone());
                    values.push(itf.clone());
                }
            }
        }
    }

    let selection = dialoguer::Select::new()
        .with_prompt("Which interface do you want to import?")
        .items(&labels)
        .interact()
        .context("Failed to select interface")?;

    Ok(values[selection].clone())
}

/// Collect the capability sets the dependency requires, inferred from its imports.
fn collect_required_capabilities(wasm_bytes: &[u8]) -> Result<Vec<String>> {
    Ok(
        match spin_capabilities::InheritConfiguration::collect(wasm_bytes)
            .context("Failed to collect capability requirements from the dependency")?
        {
            Some(spin_capabilities::InheritConfiguration::Some(caps)) => caps,
            _ => vec![],
        },
    )
}

/// Determine the `inherit_configuration` value for the dependency (interactive).
///
/// Returns the value to record (never `All(true)` — always an explicit list) and
/// the full set of capabilities the dependency requires (for post-add guidance).
fn resolve_inherit_configuration(
    manifest: &AppManifest,
    component_id: &str,
    wasm_bytes: &[u8],
) -> Result<(Option<InheritConfiguration>, Vec<String>)> {
    let required = collect_required_capabilities(wasm_bytes)?;
    if required.is_empty() {
        return Ok((None, vec![]));
    }

    // Edge case: if the component already sets the blanket
    // `dependencies_inherit_configuration`, that covers all dependencies, so we
    // neither prompt nor write a per-dependency `inherit_configuration`.
    if let Ok(kebab) = TryInto::<KebabId>::try_into(component_id.to_string())
        && let Some(component) = manifest.components.get(&kebab)
        && component.dependencies_inherit_configuration.is_some()
    {
        return Ok((None, required));
    }

    println!(
        "This dependency requires the following capabilities: {}",
        required.join(", ")
    );

    let selections = dialoguer::MultiSelect::new()
        .with_prompt("Select capabilities to inherit from the parent component")
        .items(&required)
        .interact()
        .context("Failed to select capabilities")?;

    if selections.is_empty() {
        return Ok((None, required));
    }

    // Always record the explicit list of selected capabilities. We deliberately
    // never emit `inherit_configuration = true`, even when every listed
    // capability is selected, so that a future version of the dependency that
    // imports a new capability does not silently inherit it.
    let selected: Vec<String> = selections
        .into_iter()
        .map(|i| required[i].clone())
        .collect();
    Ok((Some(InheritConfiguration::Some(selected)), required))
}

/// Print guidance about the capabilities the dependency needs.
fn print_capability_guidance(
    component_id: &str,
    required_caps: &[String],
    inherit_config: Option<&InheritConfiguration>,
) {
    if required_caps.is_empty() {
        return;
    }

    let inherited: Vec<String> = match inherit_config {
        Some(InheritConfiguration::Some(list)) => list.clone(),
        Some(InheritConfiguration::All(_)) => required_caps.to_vec(),
        None => vec![],
    };
    let declined: Vec<String> = required_caps
        .iter()
        .filter(|c| !inherited.contains(c))
        .cloned()
        .collect();

    println!();
    if !inherited.is_empty() {
        println!("NOTE: This dependency inherits: {}.", inherited.join(", "));
        println!(
            "Ensure component '{component_id}' declares these capabilities so the dependency can use them."
        );
    }
    if !declined.is_empty() {
        println!(
            "NOTE: The dependency also uses {} which was not inherited; it will be denied these at runtime.",
            declined.join(", ")
        );
    }
}

/// Build the `ComponentDependency` value from the resolved source.
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
        ResolvedSource::Component { id } => {
            let component: KebabId = id
                .clone()
                .try_into()
                .map_err(|e| anyhow!("'{id}' is not a valid component id: {e}"))?;
            Ok(ComponentDependency::AppComponent {
                component,
                export: None,
                inherit_configuration: inherit_config,
            })
        }
    }
}

/// Write the dependency into the spin.toml manifest, preserving formatting.
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

    // Navigate to [component.<component_id>].
    let component_table = doc
        .get_mut("component")
        .and_then(|c| c.as_table_like_mut())
        .context("No [component] table in manifest")?;

    let component = component_table
        .get_mut(component_id)
        .and_then(|c| c.as_table_like_mut())
        .with_context(|| format!("Component '{component_id}' not found in manifest"))?;

    // Ensure [component.<id>.dependencies] exists.
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

    let dep_toml_value = serialize_component_dependency(dep_value)?;
    deps_table.insert(&dep_key, dep_toml_value);

    std::fs::write(manifest_file, doc.to_string()).context("Failed to write manifest file")?;

    Ok(())
}

/// Serialize a `ComponentDependency` into a `toml_edit::Item`.
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

/// The HTTP route of a trigger, if it has one (private endpoints do not).
fn trigger_route(trigger: &Trigger) -> Option<&str> {
    trigger.config.get("route").and_then(|v| v.as_str())
}

/// A short human-readable label for an existing middleware entry.
fn trigger_dependency_label(dep: &TriggerDependency) -> String {
    match dep {
        TriggerDependency::Package {
            package, version, ..
        } => format!("{package}@{version}"),
        TriggerDependency::Local { path, .. } => path.display().to_string(),
        TriggerDependency::HTTP { url, .. } => url.clone(),
        TriggerDependency::AppComponent { component, .. } => component.as_ref().to_string(),
    }
}

/// A short label for the source being added (for the confirmation message).
fn source_label(source: &ResolvedSource) -> String {
    match source {
        ResolvedSource::Local { path } => path.display().to_string(),
        ResolvedSource::Http { url, .. } => url.clone(),
        ResolvedSource::Registry {
            package, version, ..
        } => match package {
            Some(p) => format!("{p}@{version}"),
            None => version.clone(),
        },
        ResolvedSource::Component { id } => id.clone(),
    }
}

/// Serialize the resolved source as a middleware entry inline table.
fn serialize_trigger_dependency(
    source: &ResolvedSource,
    inherit: Option<&InheritConfiguration>,
) -> toml_edit::Value {
    let mut table = toml_edit::InlineTable::new();
    match source {
        ResolvedSource::Local { path } => {
            table.insert(
                "path",
                toml_edit::Value::from(path.to_string_lossy().as_ref()),
            );
        }
        ResolvedSource::Http { url, digest } => {
            table.insert("url", toml_edit::Value::from(url.as_str()));
            table.insert("digest", toml_edit::Value::from(digest.as_str()));
        }
        ResolvedSource::Registry {
            version,
            registry,
            package,
        } => {
            table.insert("version", toml_edit::Value::from(version.as_str()));
            if let Some(package) = package {
                table.insert("package", toml_edit::Value::from(package.as_str()));
            }
            if let Some(registry) = registry {
                table.insert("registry", toml_edit::Value::from(registry.as_str()));
            }
        }
        ResolvedSource::Component { id } => {
            table.insert("component", toml_edit::Value::from(id.as_str()));
        }
    }
    insert_inherit_configuration(&mut table, &inherit.cloned());
    toml_edit::Value::InlineTable(table)
}

/// Insert a middleware entry into the matching HTTP trigger's pipeline.
fn write_middleware_to_manifest(
    manifest_file: &std::path::Path,
    route: &str,
    index: usize,
    entry: toml_edit::Value,
) -> Result<()> {
    use toml_edit::{Array, DocumentMut, Item, Table, Value};

    let manifest_text =
        std::fs::read_to_string(manifest_file).context("Failed to read manifest file")?;
    let mut doc: DocumentMut = manifest_text
        .parse()
        .context("Failed to parse manifest as TOML")?;

    let http = doc
        .get_mut("trigger")
        .and_then(|t| t.get_mut("http"))
        .context("The manifest has no [[trigger.http]] entries")?;
    let triggers = http
        .as_array_of_tables_mut()
        .context("Expected [[trigger.http]] to be an array of tables")?;

    let table = triggers
        .iter_mut()
        .find(|t| t.get("route").and_then(|r| r.as_str()) == Some(route))
        .with_context(|| format!("No HTTP trigger found with route '{route}'"))?;

    if table.get("dependencies").is_none() {
        // Render as `dependencies.middleware = [...]` to match the manifest style.
        let mut deps = Table::new();
        deps.set_dotted(true);
        table.insert("dependencies", Item::Table(deps));
    }
    let deps = table
        .get_mut("dependencies")
        .and_then(|d| d.as_table_mut())
        .context("Failed to access the trigger's dependencies table")?;
    if deps.get("middleware").is_none() {
        deps.insert("middleware", Item::Value(Value::Array(Array::new())));
    }
    let middleware = deps
        .get_mut("middleware")
        .and_then(|m| m.as_array_mut())
        .context("Failed to access the middleware array")?;

    let idx = index.min(middleware.len());
    middleware.insert(idx, entry);

    std::fs::write(manifest_file, doc.to_string()).context("Failed to write manifest file")?;

    Ok(())
}

/// Print guidance about the capabilities the middleware needs.
fn print_middleware_capability_guidance(
    component_id: Option<&str>,
    required_caps: &[String],
    inherit_config: Option<&InheritConfiguration>,
) {
    if required_caps.is_empty() {
        return;
    }

    let inherited: Vec<String> = match inherit_config {
        Some(InheritConfiguration::Some(list)) => list.clone(),
        Some(InheritConfiguration::All(_)) => required_caps.to_vec(),
        None => vec![],
    };
    let declined: Vec<String> = required_caps
        .iter()
        .filter(|c| !inherited.contains(c))
        .cloned()
        .collect();

    println!();
    if !inherited.is_empty() {
        match component_id {
            Some(cid) => println!(
                "NOTE: This middleware inherits: {}. Ensure component '{cid}' (served by this route) declares these capabilities.",
                inherited.join(", ")
            ),
            None => println!(
                "NOTE: This middleware inherits: {}. Ensure the component served by this route declares these capabilities.",
                inherited.join(", ")
            ),
        }
    }
    if !declined.is_empty() {
        println!(
            "NOTE: The middleware also uses {} which was not inherited; it will be denied these at runtime.",
            declined.join(", ")
        );
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
        assert_eq!(id, "ensure-admin");
    }

    #[test]
    fn invalid_registry_source_errors() {
        // A namespaced-looking token that is not a valid package reference should
        // error rather than be misclassified.
        assert!("my:@@bad".parse::<ParsedSource>().is_err());
    }
}
