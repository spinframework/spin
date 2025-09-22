// Needed for clap derive: https://github.com/clap-rs/clap/issues/4857
#![allow(clippy::almost_swapped)]

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use semver::Version;
use spin_plugins::{
    error::Error,
    lookup::{fetch_plugins_repo, plugins_repo_url, PluginLookup},
    manager::{self, InstallAction, ManifestLocation, PluginManager},
    manifest::{PluginManifest, PluginPackage},
};
use std::path::{Path, PathBuf};
use url::Url;

use crate::build_info::*;
use crate::opts::*;

/// Install/uninstall Spin plugins.
#[derive(Subcommand, Debug)]
pub enum PluginCommands {
    /// Install plugin from a manifest.
    ///
    /// The binary file and manifest of the plugin is copied to the local Spin
    /// plugins directory.
    Install(Install),

    /// List available or installed plugins.
    List(List),

    /// Search for plugins by name.
    Search(Search),

    /// Remove a plugin from your installation.
    #[clap(alias = "rm")]
    Uninstall(Uninstall),

    /// Upgrade one or all plugins.
    Upgrade(Upgrade),

    /// Fetch the latest Spin plugins from the spin-plugins repository.
    Update,

    /// Print information about a plugin.
    Show(Show),
}

impl PluginCommands {
    pub async fn run(self) -> Result<()> {
        match self {
            PluginCommands::Install(cmd) => cmd.run().await,
            PluginCommands::List(cmd) => cmd.run().await,
            PluginCommands::Search(cmd) => cmd.run().await,
            PluginCommands::Uninstall(cmd) => cmd.run().await,
            PluginCommands::Upgrade(cmd) => cmd.run().await,
            PluginCommands::Update => update().await,
            PluginCommands::Show(cmd) => cmd.run().await,
        }
    }
}

/// Install plugins from remote source
#[derive(Parser, Debug)]
pub struct Install {
    /// Name of Spin plugin.
    #[clap(
        name = PLUGIN_NAME_OPT,
        conflicts_with = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        required_unless_present_any = [PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT, PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT],
    )]
    pub name: Option<String>,

    /// Path to local plugin manifest.
    #[clap(
        name = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        short = 'f',
        long = "file",
        conflicts_with = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_NAME_OPT,
        value_hint = clap::ValueHint::FilePath,
    )]
    pub local_manifest_src: Option<PathBuf>,

    /// URL of remote plugin manifest to install.
    #[clap(
        name = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        short = 'u',
        long = "url",
        conflicts_with = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_NAME_OPT,
    )]
    pub remote_manifest_src: Option<Url>,

    /// Skips prompt to accept the installation of the plugin.
    #[clap(short = 'y', long = "yes", takes_value = false)]
    pub yes_to_all: bool,

    /// Overrides a failed compatibility check of the plugin with the current version of Spin.
    #[clap(long = PLUGIN_OVERRIDE_COMPATIBILITY_CHECK_FLAG, takes_value = false)]
    pub override_compatibility_check: bool,

    /// Provide the value for the authorization header to be able to install a plugin from a private repository.
    /// (e.g) --auth-header-value "Bearer <token>"
    #[clap(long = "auth-header-value", requires = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT)]
    pub auth_header_value: Option<String>,

    /// Specific version of a plugin to be install from the centralized plugins
    /// repository.
    #[clap(
        long = "version",
        short = 'v',
        conflicts_with = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        requires(PLUGIN_NAME_OPT)
    )]
    pub version: Option<Version>,
}

impl Install {
    pub async fn run(&self) -> Result<()> {
        let manifest_location = match (&self.local_manifest_src, &self.remote_manifest_src, &self.name) {
            (Some(path), None, None) => ManifestLocation::Local(path.to_path_buf()),
            (None, Some(url), None) => ManifestLocation::Remote(url.clone()),
            (None, None, Some(name)) => ManifestLocation::PluginsRepository(PluginLookup::new(name, self.version.clone())),
            _ => return Err(anyhow::anyhow!("For plugin lookup, must provide exactly one of: plugin name, url to manifest, local path to manifest")),
        };
        let manager = PluginManager::try_default()?;
        // Downgrades are only allowed via the `upgrade` subcommand
        let downgrade = false;
        let manifest = manager
            .get_manifest(
                &manifest_location,
                self.override_compatibility_check,
                SPIN_VERSION,
                &self.auth_header_value,
            )
            .await?;
        try_install(
            &manifest,
            &manager,
            self.yes_to_all,
            self.override_compatibility_check,
            downgrade,
            &manifest_location,
            &self.auth_header_value,
        )
        .await?;
        Ok(())
    }
}

/// Uninstalls specified plugin.
#[derive(Parser, Debug)]
pub struct Uninstall {
    /// Name of Spin plugin.
    pub name: String,
}

impl Uninstall {
    pub async fn run(self) -> Result<()> {
        let manager = PluginManager::try_default()?;
        let uninstalled = manager.uninstall(&self.name)?;
        if uninstalled {
            println!("Plugin {} was successfully uninstalled", self.name);
        } else {
            println!(
                "Plugin {} isn't present, so no changes were made",
                self.name
            );
        }
        Ok(())
    }
}

#[derive(Parser, Debug)]
pub struct Upgrade {
    /// Name of Spin plugin to upgrade.
    #[clap(
        name = PLUGIN_NAME_OPT,
        conflicts_with = PLUGIN_ALL_OPT,
    )]
    pub name: Option<String>,

    /// Upgrade all plugins.
    #[clap(
        short = 'a',
        long = "all",
        name = PLUGIN_ALL_OPT,
        conflicts_with = PLUGIN_NAME_OPT,
        conflicts_with = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        takes_value = false,
    )]
    pub all: bool,

    /// Path to local plugin manifest.
    #[clap(
        name = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        short = 'f',
        long = "file",
        conflicts_with = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        value_hint = clap::ValueHint::AnyPath,
    )]
    pub local_manifest_src: Option<PathBuf>,

    /// Path to remote plugin manifest.
    #[clap(
        name = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        short = 'u',
        long = "url",
        conflicts_with = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
    )]
    pub remote_manifest_src: Option<Url>,

    /// Skips prompt to accept the installation of the plugin[s].
    #[clap(short = 'y', long = "yes", takes_value = false)]
    pub yes_to_all: bool,

    /// Provide the value for the authorization header to be able to install a plugin from a private repository.
    /// (e.g) --auth-header-value "Bearer <token>"
    #[clap(long = "auth-header-value", requires = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT)]
    pub auth_header_value: Option<String>,

    /// Overrides a failed compatibility check of the plugin with the current version of Spin.
    #[clap(long = PLUGIN_OVERRIDE_COMPATIBILITY_CHECK_FLAG, takes_value = false)]
    pub override_compatibility_check: bool,

    /// Specific version of a plugin to be install from the centralized plugins
    /// repository.
    #[clap(
        long = "version",
        short = 'v',
        conflicts_with = PLUGIN_REMOTE_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_LOCAL_PLUGIN_MANIFEST_OPT,
        conflicts_with = PLUGIN_ALL_OPT,
        requires(PLUGIN_NAME_OPT)
    )]
    pub version: Option<Version>,

    /// Allow downgrading a plugin's version.
    #[clap(short = 'd', long = "downgrade", takes_value = false)]
    pub downgrade: bool,
}

impl Upgrade {
    /// Upgrades one or all plugins by reinstalling the latest or a specified
    /// version of a plugin. If downgrade is specified, first uninstalls the
    /// plugin.
    /// Also, by default, Spin displays the list of installed plugins that are in
    /// the catalogue and prompts user to choose which ones to upgrade.
    pub async fn run(self) -> Result<()> {
        let manager = PluginManager::try_default()?;
        let manifests_dir = manager.store().installed_manifests_directory();

        // Check if no plugins are currently installed
        if !manifests_dir.exists() {
            println!("No currently installed plugins to upgrade.");
            return Ok(());
        }

        if self.all {
            self.upgrade_all(manifests_dir).await
        } else if self.name.is_none()
            && self.local_manifest_src.is_none()
            && self.remote_manifest_src.is_none()
        {
            // Default behavior (multiselect)
            self.upgrade_multiselect().await
        } else {
            self.upgrade_one().await
        }
    }

    // Multiselect plugin upgrade experience
    async fn upgrade_multiselect(self) -> Result<()> {
        let catalogue_plugins = list_catalogue_plugins().await?;
        let installed_plugins = list_installed_plugins()?;

        let installed_in_catalogue: Vec<_> = installed_plugins
            .into_iter()
            .filter(|installed| {
                catalogue_plugins
                    .iter()
                    .any(|catalogue| installed.manifest.name() == catalogue.manifest.name())
            })
            .collect();

        if installed_in_catalogue.is_empty() {
            eprintln!("No plugins found to upgrade");
            return Ok(());
        }

        let mut eligible_plugins = Vec::new();

        // Getting only eligible plugins to upgrade
        for installed_plugin in installed_in_catalogue {
            let manager = PluginManager::try_default()?;
            let manifest_location = ManifestLocation::PluginsRepository(PluginLookup::new(
                &installed_plugin.name,
                None,
            ));

            // Attempt to get the manifest to check eligibility to upgrade
            if let Ok(manifest) = manager
                .get_manifest(
                    &manifest_location,
                    false,
                    SPIN_VERSION,
                    &self.auth_header_value,
                )
                .await
            {
                // Check if upgraded candidates have a newer version and if are compatible
                if is_potential_upgrade(&installed_plugin.manifest, &manifest)
                    && PluginCompatibility::Compatible
                        == PluginCompatibility::for_current(&manifest)
                {
                    eligible_plugins.push((installed_plugin, manifest));
                }
            }
        }

        if eligible_plugins.is_empty() {
            eprintln!("All plugins are up to date");
            return Ok(());
        }

        let names: Vec<_> = eligible_plugins
            .iter()
            .map(|(descriptor, manifest)| {
                format!(
                    "{} from version {} to {}",
                    descriptor.name,
                    descriptor.version,
                    manifest.version()
                )
            })
            .collect();

        eprintln!(
            "Select plugins to upgrade. Use Space to select/deselect and Enter to confirm selection."
        );
        let selected_indexes = match dialoguer::MultiSelect::new().items(&names).interact_opt()? {
            Some(indexes) => indexes,
            None => return Ok(()),
        };

        let plugins_selected = elements_at(eligible_plugins, selected_indexes);

        if plugins_selected.is_empty() {
            eprintln!("No plugins selected");
            return Ok(());
        }

        // Upgrade plugins selected
        for (installed_plugin, manifest) in plugins_selected {
            let manager = PluginManager::try_default()?;
            let manifest_location = ManifestLocation::PluginsRepository(PluginLookup::new(
                &installed_plugin.name,
                None,
            ));

            try_install(
                &manifest,
                &manager,
                true,
                false,
                false,
                &manifest_location,
                &self.auth_header_value,
            )
            .await?;
        }

        Ok(())
    }

    // Install the latest of all currently installed plugins
    async fn upgrade_all(&self, manifests_dir: impl AsRef<Path>) -> Result<()> {
        let manager = PluginManager::try_default()?;
        for plugin in std::fs::read_dir(manifests_dir)? {
            let path = plugin?.path();
            let name = path
                .file_stem()
                .ok_or_else(|| anyhow!("No stem for path {}", path.display()))?
                .to_str()
                .ok_or_else(|| anyhow!("Cannot convert path {} stem to str", path.display()))?
                .to_string();
            let manifest_location =
                ManifestLocation::PluginsRepository(PluginLookup::new(&name, None));
            let manifest = match manager
                .get_manifest(
                    &manifest_location,
                    self.override_compatibility_check,
                    SPIN_VERSION,
                    &self.auth_header_value,
                )
                .await
            {
                Err(Error::NotFound(e)) => {
                    tracing::info!("Could not upgrade plugin '{name}': {e:?}");
                    continue;
                }
                Err(e) => return Err(e.into()),
                Ok(m) => m,
            };
            try_install(
                &manifest,
                &manager,
                self.yes_to_all,
                self.override_compatibility_check,
                self.downgrade,
                &manifest_location,
                &self.auth_header_value,
            )
            .await?;
        }
        Ok(())
    }

    async fn upgrade_one(self) -> Result<()> {
        let manager = PluginManager::try_default()?;
        let manifest_location = match (self.local_manifest_src, self.remote_manifest_src) {
            (Some(path), None) => ManifestLocation::Local(path),
            (None, Some(url)) => ManifestLocation::Remote(url),
            _ => ManifestLocation::PluginsRepository(PluginLookup::new(
                self.name
                    .as_ref()
                    .context("plugin name is required for upgrades")?,
                self.version,
            )),
        };
        let manifest = manager
            .get_manifest(
                &manifest_location,
                self.override_compatibility_check,
                SPIN_VERSION,
                &self.auth_header_value,
            )
            .await?;
        try_install(
            &manifest,
            &manager,
            self.yes_to_all,
            self.override_compatibility_check,
            self.downgrade,
            &manifest_location,
            &self.auth_header_value,
        )
        .await?;
        Ok(())
    }
}

#[derive(Parser, Debug)]
pub struct Show {
    /// Name of Spin plugin.
    pub name: String,
}

impl Show {
    pub async fn run(self) -> Result<()> {
        let manager = PluginManager::try_default()?;
        let manifest = manager
            .get_manifest(
                &ManifestLocation::PluginsRepository(PluginLookup::new(&self.name, None)),
                false,
                SPIN_VERSION,
                &None,
            )
            .await?;

        println!(
            "{}: {} (License: {})\n{}\n{}",
            manifest.name(),
            manifest.version(),
            manifest.license(),
            manifest
                .homepage_url()
                .map(|u| format!("{u}\n"))
                .unwrap_or_default(),
            manifest.description().unwrap_or("No description provided"),
        );
        Ok(())
    }
}

fn is_potential_upgrade(current: &PluginManifest, candidate: &PluginManifest) -> bool {
    match (current.try_version(), candidate.try_version()) {
        (Ok(cur_ver), Ok(cand_ver)) => cand_ver > cur_ver,
        _ => current.version() != candidate.version(),
    }
}

// Make list_installed_plugins and list_catalogue_plugins into 'free' module-level functions
// in order to call them in Upgrade::upgrade_multiselect
fn list_installed_plugins() -> Result<Vec<PluginDescriptor>> {
    let manager = PluginManager::try_default()?;
    let store = manager.store();
    let manifests = store.installed_manifests()?;
    let descriptors = manifests
        .into_iter()
        .map(|m| PluginDescriptor {
            name: m.name(),
            version: m.version().to_owned(),
            installed: true,
            compatibility: PluginCompatibility::for_current(&m),
            manifest: m,
            installed_version: None,
        })
        .collect();
    Ok(descriptors)
}

async fn list_catalogue_plugins() -> Result<Vec<PluginDescriptor>> {
    if update_silent().await.is_err() {
        terminal::warn!("Couldn't update plugins registry cache - using most recent");
    }

    let manager = PluginManager::try_default()?;
    let store = manager.store();
    let manifests = store.catalogue_manifests();
    let descriptors = manifests?
        .into_iter()
        .map(|m| PluginDescriptor {
            name: m.name(),
            version: m.version().to_owned(),
            installed: m.is_installed_in(store),
            compatibility: PluginCompatibility::for_current(&m),
            manifest: m,
            installed_version: None,
        })
        .collect();
    Ok(descriptors)
}

async fn list_catalogue_and_installed_plugins() -> Result<Vec<PluginDescriptor>> {
    let catalogue = list_catalogue_plugins().await?;
    let installed = list_installed_plugins()?;
    Ok(merge_plugin_lists(catalogue, installed))
}

fn summarise(all_plugins: Vec<PluginDescriptor>) -> Vec<PluginDescriptor> {
    use itertools::Itertools;

    let names_to_versions = all_plugins
        .into_iter()
        .into_group_map_by(|pd| pd.name.clone());
    names_to_versions
        .into_values()
        .flat_map(|versions| {
            let (latest, rest) = latest_and_rest(versions);
            let Some(mut latest) = latest else {
                // We can't parse things well enough to summarise: return all versions.
                return rest;
            };
            if latest.installed {
                // The installed is the latest: return it.
                return vec![latest];
            }

            let installed = rest.into_iter().find(|pd| pd.installed);
            let Some(installed) = installed else {
                // No installed version: return the latest.
                return vec![latest];
            };

            // If we get here then there is an installed version which is not the latest.
            // Mark the latest as installed (representing, in this case, that the plugin
            // is installed, even though this version isn't), and record what version _is_
            // installed.
            latest.installed = true;
            latest.installed_version = Some(installed.version);
            vec![latest]
        })
        .collect()
}

/// Given a list of plugin descriptors, this looks for the one with the latest version.
/// If it can determine a latest version, it returns a tuple where the first element is
/// the latest version, and the second is the remaining versions (order not preserved).
/// Otherwise it returns None and the original list.
fn latest_and_rest(
    mut plugins: Vec<PluginDescriptor>,
) -> (Option<PluginDescriptor>, Vec<PluginDescriptor>) {
    // `versions` is the parsed version of each plugin in the vector, in the same order.
    // We rely on this 1-1 order-preserving behaviour as we are going to calculate
    // an index from `versions` and use it to index into `plugins`.
    let Ok(versions) = plugins
        .iter()
        .map(|pd| semver::Version::parse(&pd.version))
        .collect::<Result<Vec<_>, _>>()
    else {
        return (None, plugins);
    };
    let Some((latest_index, _)) = versions.iter().enumerate().max_by_key(|(_, v)| *v) else {
        return (None, plugins);
    };
    let pd = plugins.swap_remove(latest_index);
    (Some(pd), plugins)
}

/// List available or installed plugins.
#[derive(Parser, Debug)]
pub struct List {
    /// List only installed plugins.
    #[clap(long = "installed", takes_value = false, group = "which")]
    pub installed: bool,

    /// List all versions of plugins. This is the default behaviour.
    #[clap(long = "all", takes_value = false, group = "which")]
    pub all: bool,

    /// List latest and installed versions of plugins.
    #[clap(long = "summary", takes_value = false, group = "which")]
    pub summary: bool,

    /// Filter the list to plugins containing this string.
    #[clap(long = "filter")]
    pub filter: Option<String>,

    /// The format in which to list the templates.
    #[clap(value_enum, long = "format", default_value = "plain")]
    pub format: ListFormat,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum ListFormat {
    Plain,
    Json,
    NamesOnly,
}

impl List {
    pub async fn run(self) -> Result<()> {
        let mut plugins = if self.installed {
            list_installed_plugins()
        } else {
            list_catalogue_and_installed_plugins().await
        }?;

        if self.summary {
            plugins = summarise(plugins);
        }

        plugins.sort_by(|p, q| p.cmp(q));

        if let Some(filter) = self.filter.as_ref() {
            plugins.retain(|p| p.name.contains(filter));
        }

        match self.format {
            ListFormat::Plain => Self::print_plain(&plugins),
            ListFormat::Json => Self::print_json(&plugins),
            ListFormat::NamesOnly => Self::print_names_only(&plugins),
        }
    }

    fn print_plain(plugins: &[PluginDescriptor]) -> anyhow::Result<()> {
        if plugins.is_empty() {
            println!("No plugins found");
        } else {
            for p in plugins {
                let installed = if p.installed {
                    if let Some(installed) = p.installed_version.as_ref() {
                        format!(" [installed version: {installed}]")
                    } else {
                        " [installed]".to_string()
                    }
                } else {
                    "".to_string()
                };
                let compat = match &p.compatibility {
                    PluginCompatibility::Compatible => String::new(),
                    PluginCompatibility::IncompatibleSpin(v) => format!(" [requires Spin {v}]"),
                    PluginCompatibility::Incompatible => String::from(" [incompatible]"),
                };
                println!("{} {}{}{}", p.name, p.version, installed, compat);
            }
        }

        Ok(())
    }

    fn print_json(plugins: &[PluginDescriptor]) -> anyhow::Result<()> {
        let json_vals: Vec<_> = plugins.iter().map(json_list_format).collect();

        let json_text = serde_json::to_string_pretty(&json_vals)?;
        println!("{json_text}");
        Ok(())
    }

    fn print_names_only(plugins: &[PluginDescriptor]) -> anyhow::Result<()> {
        let names: std::collections::HashSet<_> = plugins.iter().map(|p| &p.name).collect();

        for name in names {
            println!("{name}");
        }

        Ok(())
    }
}

/// Search for plugins by name.
#[derive(Parser, Debug)]
pub struct Search {
    /// The text to search for. If omitted, all plugins are returned.
    pub filter: Option<String>,

    /// The format in which to list the plugins.
    #[clap(value_enum, long = "format", default_value = "plain")]
    pub format: ListFormat,
}

impl Search {
    async fn run(&self) -> anyhow::Result<()> {
        let list_cmd = List {
            installed: false,
            all: true,
            summary: false,
            filter: self.filter.clone(),
            format: self.format.clone(),
        };

        list_cmd.run().await
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum PluginCompatibility {
    Compatible,
    IncompatibleSpin(String),
    Incompatible,
}

impl PluginCompatibility {
    pub(crate) fn for_current(manifest: &PluginManifest) -> Self {
        if manifest.has_compatible_package() {
            let spin_version = SPIN_VERSION;
            if manifest.is_compatible_spin_version(spin_version) {
                Self::Compatible
            } else {
                Self::IncompatibleSpin(manifest.spin_compatibility())
            }
        } else {
            Self::Incompatible
        }
    }
}

#[derive(Debug)]
struct PluginDescriptor {
    name: String,
    version: String,
    compatibility: PluginCompatibility,
    installed: bool,
    manifest: PluginManifest,
    installed_version: Option<String>, // only in "latest" mode and if installed version is not latest
}

impl PluginDescriptor {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let version_cmp = match (
            semver::Version::parse(&self.version),
            semver::Version::parse(&other.version),
        ) {
            (Ok(v1), Ok(v2)) => v1.cmp(&v2),
            _ => self.version.cmp(&other.version),
        };

        self.name.cmp(&other.name).then(version_cmp)
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginJsonFormat {
    name: String,
    installed: bool,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    installed_version: Option<String>,
}

fn json_list_format(plugin: &PluginDescriptor) -> PluginJsonFormat {
    let installed_version = if plugin.installed {
        Some(
            plugin
                .installed_version
                .clone()
                .unwrap_or_else(|| plugin.version.clone()),
        )
    } else {
        None
    };

    PluginJsonFormat {
        name: plugin.name.clone(),
        installed: plugin.installed,
        version: plugin.version.clone(),
        installed_version,
    }
}

// Auxiliar function for Upgrade::upgrade_multiselect
fn elements_at<T>(source: Vec<T>, indexes: Vec<usize>) -> Vec<T> {
    source
        .into_iter()
        .enumerate()
        .filter_map(|(index, s)| {
            if indexes.contains(&index) {
                Some(s)
            } else {
                None
            }
        })
        .collect()
}

fn merge_plugin_lists(a: Vec<PluginDescriptor>, b: Vec<PluginDescriptor>) -> Vec<PluginDescriptor> {
    let mut result = a;

    for descriptor in b {
        // Use the manifest for sameness checking, because an installed local build could have the same name
        // and version as a registry package, yet be a different binary. It could even have different
        // compatibility characteristics!
        let already_got = result
            .iter()
            .any(|desc| desc.manifest == descriptor.manifest);
        if !already_got {
            result.push(descriptor);
        }
    }

    result
}

/// Updates the locally cached spin-plugins repository, fetching the latest plugins.
pub(crate) async fn update() -> Result<()> {
    update_silent().await?;
    println!("Plugin information updated successfully");
    Ok(())
}

pub(crate) async fn update_silent() -> Result<()> {
    let manager = PluginManager::try_default()?;

    let mut locker = manager.update_lock().await;
    let guard = locker.lock_updates();
    if guard.denied() {
        anyhow::bail!("Another plugin update operation is already in progress");
    }

    let plugins_dir = manager.store().get_plugins_directory();
    let url = plugins_repo_url()?;
    fetch_plugins_repo(&url, plugins_dir, true).await?;
    Ok(())
}

fn continue_to_install(
    manifest: &PluginManifest,
    package: &PluginPackage,
    yes_to_all: bool,
) -> Result<bool> {
    Ok(yes_to_all || prompt_confirm_install(manifest, package)?)
}

fn prompt_confirm_install(manifest: &PluginManifest, package: &PluginPackage) -> Result<bool> {
    println!(
        "You are trying to install the `{}` plugin with {} license from {} ",
        manifest.name(),
        manifest.license(),
        package.url()
    );
    let prompt = "Are you sure you want to continue?".to_string();
    let install = dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(true)
        .interact_opt()?
        .unwrap_or(false);
    if !install {
        println!("Plugin '{}' will not be installed", manifest.name());
    }
    Ok(install)
}

async fn try_install(
    manifest: &PluginManifest,
    manager: &PluginManager,
    yes_to_all: bool,
    override_compatibility_check: bool,
    downgrade: bool,
    source: &ManifestLocation,
    auth_header_value: &Option<String>,
) -> Result<bool> {
    let install_action = manager.check_manifest(
        manifest,
        SPIN_VERSION,
        override_compatibility_check,
        downgrade,
    )?;

    if let InstallAction::NoAction { name, version } = install_action {
        eprintln!("Plugin '{name}' is already installed with version {version}.");
        return Ok(false);
    }

    let package = manager::get_package(manifest)?;
    if continue_to_install(manifest, package, yes_to_all)? {
        let installed = manager
            .install(manifest, package, source, auth_header_value)
            .await?;
        println!("Plugin '{installed}' was installed successfully!");

        if let Some(description) = manifest.description() {
            println!("\nDescription:");
            println!("\t{description}");
        }

        if let Some(homepage) = manifest.homepage_url().filter(|h| h.scheme() == "https") {
            println!("\nHomepage:");
            println!("\t{homepage}");
        }

        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn dummy_descriptor(version: &str) -> PluginDescriptor {
        use serde::Deserialize;
        PluginDescriptor {
            name: "dummy".into(),
            version: version.into(),
            compatibility: PluginCompatibility::Compatible,
            installed: false,
            manifest: PluginManifest::deserialize(serde_json::json!({
                "name": "dummy",
                "version": version,
                "spinCompatibility": ">= 0.1",
                "license": "dummy",
                "packages": []
            }))
            .unwrap(),
            installed_version: None,
        }
    }

    #[test]
    fn latest_and_rest_if_empty_returns_no_latest_rest_empty() {
        let (latest, rest) = latest_and_rest(vec![]);
        assert!(latest.is_none());
        assert_eq!(0, rest.len());
    }

    #[test]
    fn latest_and_rest_if_invalid_ver_returns_no_latest_all_rest() {
        let (latest, rest) = latest_and_rest(vec![
            dummy_descriptor("1.2.3"),
            dummy_descriptor("spork"),
            dummy_descriptor("1.3.5"),
        ]);
        assert!(latest.is_none());
        assert_eq!(3, rest.len());
    }

    #[test]
    fn latest_and_rest_if_valid_ver_returns_latest_and_rest() {
        let (latest, rest) = latest_and_rest(vec![
            dummy_descriptor("1.2.3"),
            dummy_descriptor("2.4.6"),
            dummy_descriptor("1.3.5"),
        ]);
        let latest = latest.expect("should have found a latest");
        assert_eq!("2.4.6", latest.version);

        assert_eq!(2, rest.len());
        let rest_vers: std::collections::HashSet<_> = rest.into_iter().map(|p| p.version).collect();
        assert!(rest_vers.contains("1.2.3"));
        assert!(rest_vers.contains("1.3.5"));
    }
}
