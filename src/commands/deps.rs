mod source;

use crate::opts::*;
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use source::{ParsedSource, ResolvedSource};
use spin_common::paths::parent_dir;
use spin_manifest::schema::v2::{
    AppManifest, Component, ComponentDependency, ComponentSpec, InheritConfiguration, Trigger,
    TriggerDependency,
};
use spin_serde::{DependencyName, DependencyPackageName, KebabId};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The manifest key under which a component's dependencies live.
const DEPENDENCIES_KEY: &str = "dependencies";
/// The trigger-dependencies key under which HTTP middleware entries live.
const MIDDLEWARE_KEY: &str = "middleware";

/// Commands for managing component dependencies.
#[derive(Subcommand, Debug)]
pub enum DependenciesCommands {
    /// Add a component dependency to a component in the application.
    Add(AddCommand),
}

impl DependenciesCommands {
    pub async fn run(self) -> Result<()> {
        match self {
            DependenciesCommands::Add(cmd) => cmd.run().await,
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

impl AddCommand {
    pub async fn run(self) -> Result<()> {
        // Fail fast on a bad invocation: these flows are interactive, and the
        // source options must make sense before we go near the manifest.
        ensure_interactive()?;
        self.source
            .validate_options(self.digest.as_deref(), self.registry.as_deref())?;

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

        // Resolve the source to Wasm bytes plus the metadata needed to record it,
        // then inspect the bytes once for everything the steps below need.
        let (wasm_bytes, dep_source) = self
            .source
            .resolve(
                self.digest.as_deref(),
                self.registry.as_deref(),
                &app_root,
                &manifest,
            )
            .await?;
        let interfaces = spin_dependency_wit::ComponentInterfaces::from_component(&wasm_bytes)
            .context("Failed to inspect the component's interfaces")?;
        let required_caps = collect_required_capabilities(&wasm_bytes)?;

        // Dispatch on what we're adding: a component that both imports and exports
        // wasi:http/handler is HTTP middleware (attached to a trigger); anything
        // else is a component dependency (attached to a component).
        if interfaces.is_http_middleware() {
            add_middleware(&manifest_file, &manifest, required_caps, dep_source)
        } else {
            add_component_dependency(
                &manifest_file,
                &app_root,
                &manifest,
                &interfaces,
                required_caps,
                dep_source,
            )
            .await
        }
    }
}

/// Add the resolved source as a component dependency to a selected component.
async fn add_component_dependency(
    manifest_file: &Path,
    app_root: &Path,
    manifest: &AppManifest,
    interfaces: &spin_dependency_wit::ComponentInterfaces,
    required_caps: Vec<&'static str>,
    dep_source: ResolvedSource,
) -> Result<()> {
    let Some((component_id, component)) = select_target_component(manifest)? else {
        return cancelled();
    };
    let Some(selected) = select_interface(interfaces)? else {
        return cancelled();
    };
    let dep_name: DependencyName = selected.parse().with_context(|| {
        format!("Failed to parse selected interface '{selected}' as a dependency name")
    })?;

    // Check for an existing entry before prompting about capabilities, so we
    // don't badger the user over an entry we can't write.
    ensure_dependency_absent(component, component_id, &dep_name)?;

    let target = format!("component '{component_id}'");
    let Some(inheritance) = select_inheritance(required_caps, Some(component), &target)? else {
        return cancelled();
    };

    let dep_value = dep_source.to_component_dependency(inheritance.to_write.clone(), None);
    write_dependency_to_manifest(manifest_file, component_id, &dep_name, &dep_value)?;
    regenerate_dependencies_wit(manifest_file, app_root, component_id).await?;

    println!("Added {selected} to {target}");
    println!("Run `spin build` to generate language bindings for the new dependency.");
    print_capability_guidance(&target, Some(component), &inheritance);

    Ok(())
}

/// Attach the resolved source as HTTP middleware to a selected trigger.
fn add_middleware(
    manifest_file: &Path,
    manifest: &AppManifest,
    required_caps: Vec<&'static str>,
    dep_source: ResolvedSource,
) -> Result<()> {
    eprintln!("Detected HTTP middleware.");
    eprintln!();

    let Some(trigger) = select_trigger(manifest)? else {
        return cancelled();
    };
    let route = trigger_route(trigger)
        .context("selected trigger has no route")?
        .to_string();
    let component = trigger_component(manifest, trigger);
    let target = match &component {
        Some((id, _)) => format!("component '{id}' (served by route '{route}')"),
        None => format!("the component served by route '{route}'"),
    };

    let Some(index) = select_pipeline_position(trigger)? else {
        return cancelled();
    };

    // A component's blanket `dependencies_inherit_configuration` does not cover
    // trigger middleware, so always ask.
    let Some(inheritance) = select_inheritance(required_caps, None, &target)? else {
        return cancelled();
    };

    let entry = serialize_trigger_dependency(&dep_source, inheritance.to_write.as_ref(), None);
    write_middleware_to_manifest(manifest_file, &route, index, entry)?;

    println!(
        "Added middleware '{}' to the trigger for route '{route}'",
        dep_source.label()
    );
    print_capability_guidance(&target, component.map(|(_, c)| c), &inheritance);

    Ok(())
}

/// The add flows are interactive; make sure there's a terminal to prompt on.
/// `dialoguer` prompts on stderr, so that is the stream that matters.
fn ensure_interactive() -> Result<()> {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        Ok(())
    } else {
        bail!("`spin dependencies add` is interactive and requires a terminal.");
    }
}

/// Report that the user cancelled, leaving the manifest untouched.
fn cancelled() -> Result<()> {
    eprintln!("No changes were made.");
    Ok(())
}

/// Ask which component to add the dependency to. Returns `None` if cancelled.
fn select_target_component(manifest: &AppManifest) -> Result<Option<(&KebabId, &Component)>> {
    let components: Vec<(&KebabId, &Component)> = manifest.components.iter().collect();

    if components.is_empty() {
        bail!("No components found in the manifest");
    }

    if components.len() == 1 {
        return Ok(Some(components[0]));
    }

    let ids: Vec<&str> = components.iter().map(|(id, _)| id.as_ref()).collect();
    let Some(selection) = dialoguer::Select::new()
        .with_prompt("Which component should the dependency be added to?")
        .items(&ids)
        .interact_opt()
        .context("Failed to select component")?
    else {
        return Ok(None);
    };

    Ok(Some(components[selection]))
}

/// Bail out if the component already declares `dep_name`, so we don't prompt the
/// user about capabilities for an entry we would refuse to overwrite.
fn ensure_dependency_absent(
    component: &Component,
    component_id: &KebabId,
    dep_name: &DependencyName,
) -> Result<()> {
    if component.dependencies.inner.contains_key(dep_name) {
        bail!("Dependency '{dep_name}' already exists in component '{component_id}'");
    }
    Ok(())
}

/// Regenerate `spin-dependencies.wit` for a component after its dependencies change.
async fn regenerate_dependencies_wit(
    manifest_file: &Path,
    app_root: &Path,
    component_id: &KebabId,
) -> Result<()> {
    // Reload the manifest so we pick up the dependency we just wrote.
    let manifest = spin_manifest::manifest_from_file(manifest_file)?;
    let component = manifest
        .components
        .get(component_id)
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
    .context("Failed to regenerate spin-dependencies.wit")
}

/// Ask which HTTP trigger (by route) to attach middleware to. Returns `None` if
/// the user cancels. Private (route-less) endpoints are not eligible.
fn select_trigger(manifest: &AppManifest) -> Result<Option<&Trigger>> {
    let routed: Vec<&Trigger> = manifest
        .triggers
        .get("http")
        .map(|triggers| {
            triggers
                .iter()
                .filter(|t| trigger_route(t).is_some())
                .collect()
        })
        .unwrap_or_default();

    if routed.is_empty() {
        bail!("The application has no routed HTTP triggers to attach middleware to.");
    }
    if routed.len() == 1 {
        return Ok(Some(routed[0]));
    }

    let routes: Vec<&str> = routed.iter().filter_map(|t| trigger_route(t)).collect();
    let Some(sel) = dialoguer::Select::new()
        .with_prompt("Which HTTP route should the middleware be added to?")
        .items(&routes)
        .interact_opt()
        .context("Failed to select route")?
    else {
        return Ok(None);
    };
    Ok(Some(routed[sel]))
}

/// Ask where in the trigger's middleware pipeline to insert the new entry,
/// returning the insertion index. Returns `None` if the user cancels.
fn select_pipeline_position(trigger: &Trigger) -> Result<Option<usize>> {
    let existing: Vec<String> = trigger
        .dependencies
        .get(MIDDLEWARE_KEY)
        .map(|d| d.0.iter().map(trigger_dependency_label).collect())
        .unwrap_or_default();

    if existing.is_empty() {
        return Ok(Some(0));
    }

    // List positions top-to-bottom in pipeline order: "before" each existing
    // entry, then "at the end" last, so the on-screen order matches execution
    // order (a request flows top to bottom).
    let mut items: Vec<String> = existing
        .iter()
        .map(|label| format!("Before {label}"))
        .collect();
    items.push("At the end (closest to the application component)".to_string());
    let end = items.len() - 1;

    let Some(sel) = dialoguer::Select::new()
        .with_prompt("Where should this middleware run in the pipeline?")
        .items(&items)
        .default(end)
        .interact_opt()
        .context("Failed to select pipeline position")?
    else {
        return Ok(None);
    };

    // `sel` in `0..existing.len()` inserts before that entry; the last item
    // ("at the end") appends.
    Ok(Some(sel.min(existing.len())))
}

/// The component a trigger routes to, if it is a simple reference to one defined
/// in the manifest.
fn trigger_component<'a>(
    manifest: &'a AppManifest,
    trigger: &'a Trigger,
) -> Option<(&'a KebabId, &'a Component)> {
    match &trigger.component {
        Some(ComponentSpec::Reference(id)) => manifest.components.get_key_value(id),
        _ => None,
    }
}

/// Ask which interface to import from the dependency component. Returns `None`
/// if the user cancels.
///
/// Presents a single alphabetical list of all interface exports, grouped by
/// package. For each package that exposes more than one interface, an
/// "All from <package>" entry is included so the whole package can be selected
/// as a package-level dependency.
fn select_interface(
    interfaces: &spin_dependency_wit::ComponentInterfaces,
) -> Result<Option<String>> {
    let exports = interfaces.exports();

    if exports.is_empty() {
        bail!("The Wasm component has no exports to use as a dependency");
    }

    if exports.len() == 1 {
        return Ok(Some(exports[0].clone()));
    }

    // Group exports by package. Plain-named exports (which have no package) are
    // grouped under `None`, which sorts first.
    let mut groups: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
    for export in exports {
        let pkg_key = export.parse::<DependencyPackageName>().ok().map(|p| {
            let mut key = p.package.to_string();
            if let Some(v) = &p.version {
                key.push_str(&format!("@{v}"));
            }
            key
        });
        groups.entry(pkg_key).or_default().push(export.clone());
    }

    // Build a flat list of (label, value) items. `value` is the dependency-name
    // string recorded when that item is selected.
    let mut labels: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    for (pkg_key, mut interfaces) in groups {
        interfaces.sort();
        if let Some(pkg) = pkg_key
            && interfaces.len() > 1
        {
            labels.push(format!("All from {pkg}"));
            values.push(pkg);
        }
        for itf in interfaces {
            labels.push(itf.clone());
            values.push(itf);
        }
    }

    let Some(selection) = dialoguer::Select::new()
        .with_prompt("Which interface do you want to import?")
        .items(&labels)
        .interact_opt()
        .context("Failed to select interface")?
    else {
        return Ok(None);
    };

    Ok(Some(values[selection].clone()))
}

/// Collect the capability sets the dependency requires, inferred from its imports.
fn collect_required_capabilities(wasm_bytes: &[u8]) -> Result<Vec<&'static str>> {
    Ok(spin_capabilities::required_capabilities(wasm_bytes)
        .context("Failed to collect capability requirements from the dependency")?
        .into_iter()
        .collect())
}

/// The result of deciding what capabilities a dependency may inherit.
struct Inheritance {
    /// Every capability the dependency requires (used for post-add guidance).
    required: Vec<&'static str>,
    /// The capabilities that will actually be inherited (a subset of `required`).
    inherited: Vec<&'static str>,
    /// The `inherit_configuration` value to write into the manifest entry, if any.
    ///
    /// `None` means no `inherit_configuration` key is written — either because
    /// nothing was inherited, or because the component already inherits
    /// configuration for all its dependencies via
    /// `dependencies_inherit_configuration`.
    to_write: Option<InheritConfiguration>,
}

/// Decide which of the `required` capabilities the dependency may inherit from
/// its parent (described by `parent_desc`, e.g. `"component 'api'"`), prompting
/// the user where there is a choice to make. Returns `None` if the user cancels.
///
/// We deliberately never emit `inherit_configuration = true`, even when every
/// listed capability is selected, so that a future version of the dependency that
/// imports a new capability does not silently inherit it.
fn select_inheritance(
    required: Vec<&'static str>,
    parent: Option<&Component>,
    parent_desc: &str,
) -> Result<Option<Inheritance>> {
    if required.is_empty() {
        return Ok(Some(Inheritance {
            required,
            inherited: vec![],
            to_write: None,
        }));
    }

    // If the component already inherits configuration for all its dependencies,
    // there's nothing to ask about and nothing to write — but everything is
    // inherited, so record that for the guidance message.
    if parent.is_some_and(|c| c.dependencies_inherit_configuration.is_some()) {
        return Ok(Some(Inheritance {
            inherited: required.clone(),
            required,
            to_write: None,
        }));
    }

    eprintln!(
        "This dependency uses the following capabilities: {}",
        required.join(", ")
    );
    eprintln!("If inherited, it gets the same access to them as {parent_desc}.");

    let choices = [
        "Inherit all of them",
        "Inherit none of them (the dependency will fail if it tries to use them)",
        "Choose individually",
    ];
    let Some(choice) = dialoguer::Select::new()
        .with_prompt("Which capabilities should the dependency inherit?")
        .items(&choices)
        .default(0)
        .interact_opt()
        .context("Failed to select capabilities")?
    else {
        return Ok(None);
    };

    let inherited = match choice {
        0 => required.clone(),
        1 => vec![],
        _ => match select_individual_capabilities(&required)? {
            Some(chosen) => chosen,
            None => return Ok(None),
        },
    };

    let to_write = if inherited.is_empty() {
        None
    } else {
        Some(InheritConfiguration::Some(
            inherited.iter().map(|c| c.to_string()).collect(),
        ))
    };
    Ok(Some(Inheritance {
        required,
        inherited,
        to_write,
    }))
}

/// The "choose individually" branch of [`select_inheritance`]: multi-select
/// from `required`. Returns `None` if the user cancels.
fn select_individual_capabilities(required: &[&'static str]) -> Result<Option<Vec<&'static str>>> {
    let Some(selections) = dialoguer::MultiSelect::new()
        .with_prompt("Select the capabilities to inherit")
        .items(required)
        .interact_opt()
        .context("Failed to select capabilities")?
    else {
        return Ok(None);
    };
    Ok(Some(selections.into_iter().map(|i| required[i]).collect()))
}

/// The capability sets that `component`'s manifest entry configures — that is,
/// has a non-empty value for (e.g. a non-empty `allowed_outbound_hosts` list).
/// Uses the same names as `spin_capabilities::required_capabilities` reports.
///
/// Inheriting a capability the parent has not configured is a no-op, so this
/// drives the post-add guidance.
fn configured_capabilities(component: &Component) -> Vec<&'static str> {
    let mut configured = vec![];
    if !component.ai_models.is_empty() {
        configured.push("ai_models");
    }
    if !component.allowed_outbound_hosts.is_empty() {
        configured.push("allowed_outbound_hosts");
    }
    if !component.environment.is_empty() {
        configured.push("environment");
    }
    if !component.files.is_empty() {
        configured.push("files");
    }
    if !component.key_value_stores.is_empty() {
        configured.push("key_value_stores");
    }
    if !component.sqlite_databases.is_empty() {
        configured.push("sqlite_databases");
    }
    if !component.variables.is_empty() {
        configured.push("variables");
    }
    configured
}

/// Print follow-up guidance about the dependency's capabilities: inherited
/// capabilities that the parent component has not configured yet, and required
/// capabilities that were not inherited. `target` describes the parent
/// component for the user; `parent` is its manifest entry, if known.
fn print_capability_guidance(target: &str, parent: Option<&Component>, inheritance: &Inheritance) {
    if inheritance.required.is_empty() {
        return;
    }

    let configured = parent.map(configured_capabilities).unwrap_or_default();
    let unconfigured: Vec<&str> = inheritance
        .inherited
        .iter()
        .copied()
        .filter(|c| !configured.contains(c))
        .collect();
    let declined: Vec<&str> = inheritance
        .required
        .iter()
        .copied()
        .filter(|c| !inheritance.inherited.contains(c))
        .collect();

    if !unconfigured.is_empty() {
        eprintln!();
        terminal::warn!(
            "The dependency inherits {} from {target}, but {target} has none configured in spin.toml. Configure them on {target} so the dependency can use them.",
            unconfigured.join(", ")
        );
    }
    if !declined.is_empty() {
        eprintln!();
        terminal::einfo!(
            "Note:",
            "The dependency also uses {}, which you chose not to inherit. If it tries to use them at runtime, those calls will fail.",
            declined.join(", ")
        );
    }
}

/// Write the dependency into the spin.toml manifest, preserving formatting.
fn write_dependency_to_manifest(
    manifest_file: &Path,
    component_id: &KebabId,
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
        .get_mut(component_id.as_ref())
        .and_then(|c| c.as_table_like_mut())
        .with_context(|| format!("Component '{component_id}' not found in manifest"))?;

    // Existence of the specific dependency is checked up front in
    // `ensure_dependency_absent`, so here we can insert directly.
    let deps_table = component
        .entry(DEPENDENCIES_KEY)
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .context("Failed to access dependencies table")?;

    deps_table.insert(
        &dep_name.to_string(),
        serialize_component_dependency(dep_value),
    );

    std::fs::write(manifest_file, doc.to_string()).context("Failed to write manifest file")?;

    Ok(())
}

/// Serialize a `ComponentDependency` into a `toml_edit::Item`.
///
/// This is the component-dependency analogue of [`serialize_trigger_dependency`];
/// keep the two in sync.
fn serialize_component_dependency(dep: &ComponentDependency) -> toml_edit::Item {
    let mut table = toml_edit::InlineTable::new();
    let (export, inherit_configuration, capabilities) = match dep {
        ComponentDependency::Version(version) => return toml_edit::value(version.as_str()),
        ComponentDependency::Local {
            path,
            export,
            inherit_configuration,
            capabilities,
        } => {
            table.insert(
                "path",
                toml_edit::Value::from(path.to_string_lossy().as_ref()),
            );
            (export, inherit_configuration, capabilities)
        }
        ComponentDependency::HTTP {
            url,
            digest,
            export,
            inherit_configuration,
            capabilities,
        } => {
            table.insert("url", toml_edit::Value::from(url.as_str()));
            table.insert("digest", toml_edit::Value::from(digest.as_str()));
            (export, inherit_configuration, capabilities)
        }
        ComponentDependency::Package {
            version,
            registry,
            package,
            export,
            inherit_configuration,
            capabilities,
        } => {
            table.insert("version", toml_edit::Value::from(version.as_str()));
            if let Some(registry) = registry {
                table.insert("registry", toml_edit::Value::from(registry.as_str()));
            }
            if let Some(package) = package {
                table.insert("package", toml_edit::Value::from(package.as_str()));
            }
            (export, inherit_configuration, capabilities)
        }
        ComponentDependency::AppComponent {
            component,
            export,
            inherit_configuration,
            capabilities,
        } => {
            table.insert("component", toml_edit::Value::from(component.as_ref()));
            (export, inherit_configuration, capabilities)
        }
    };
    if let Some(export) = export {
        table.insert("export", toml_edit::Value::from(export.as_str()));
    }
    insert_inherit_configuration(
        &mut table,
        inherit_configuration.as_ref(),
        capabilities.as_ref(),
    );
    toml_edit::Item::Value(toml_edit::Value::InlineTable(table))
}

/// Serialize the resolved source as an HTTP middleware entry (an inline table).
///
/// This is the trigger-dependency analogue of [`serialize_component_dependency`];
/// keep the two in sync.
fn serialize_trigger_dependency(
    source: &ResolvedSource,
    inherit: Option<&InheritConfiguration>,
    capabilities: Option<&spin_manifest::schema::v2::DependencyCapabilities>,
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
            if let Some(registry) = registry {
                table.insert("registry", toml_edit::Value::from(registry.as_str()));
            }
            table.insert("package", toml_edit::Value::from(package.as_str()));
        }
        ResolvedSource::Component { id } => {
            table.insert("component", toml_edit::Value::from(id.as_ref()));
        }
    }
    insert_inherit_configuration(&mut table, inherit, capabilities);
    toml_edit::Value::InlineTable(table)
}

fn insert_inherit_configuration(
    table: &mut toml_edit::InlineTable,
    config: Option<&InheritConfiguration>,
    capabilities: Option<&spin_manifest::schema::v2::DependencyCapabilities>,
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
    match capabilities {
        None => {}
        Some(caps) => {
            let caps_table = toml_edit::ser::to_document(caps)
                .expect("DependencyCapabilities should be serialisable");
            let caps_table = caps_table.as_table().clone().into_inline_table();
            table.insert("capabilities", toml_edit::Value::InlineTable(caps_table));
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
        TriggerDependency::AppComponent { component, .. } => component.to_string(),
    }
}

/// Insert a middleware entry into the matching HTTP trigger's pipeline.
fn write_middleware_to_manifest(
    manifest_file: &Path,
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

    let deps = table
        .entry(DEPENDENCIES_KEY)
        .or_insert_with(|| {
            // Render as `dependencies.middleware = [...]` to match the manifest style.
            let mut deps = Table::new();
            deps.set_dotted(true);
            Item::Table(deps)
        })
        .as_table_mut()
        .context("Failed to access the trigger's dependencies table")?;

    let middleware = deps
        .entry(MIDDLEWARE_KEY)
        .or_insert(Item::Value(Value::Array(Array::new())))
        .as_array_mut()
        .context("Failed to access the middleware array")?;

    let idx = index.min(middleware.len());
    middleware.insert(idx, entry);

    std::fs::write(manifest_file, doc.to_string()).context("Failed to write manifest file")?;

    Ok(())
}
