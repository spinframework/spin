use anyhow::{Context, Error};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use lazy_static::lazy_static;
use spin_cli::commands::external::predefined_externals;
use spin_cli::commands::maintenance::MaintenanceCommands;
use spin_cli::commands::{
    build::BuildCommand,
    cloud::{DeployCommand, LoginCommand},
    doctor::DoctorCommand,
    external::execute_external_subcommand,
    new::{AddCommand, NewCommand},
    plugins::PluginCommands,
    registry::RegistryCommands,
    templates::TemplateCommands,
    up::UpCommand,
    watch::WatchCommand,
};
use spin_cli::{build_info::*, subprocess::ExitStatusError};
use spin_runtime_factors::FactorsBuilder;
use spin_trigger::cli::help::HelpArgsOnlyTrigger;
use spin_trigger::cli::FactorsTriggerCommand;
use spin_trigger_http::HttpTrigger;
use spin_trigger_redis::RedisTrigger;

#[tokio::main]
async fn main() {
    if let Err(err) = _main().await {
        let code = match err.downcast_ref::<ExitStatusError>() {
            // If we encounter an `ExitStatusError` it means a subprocess has already
            // exited unsuccessfully and thus already printed error messages. No need
            // to print anything additional.
            Some(e) => e.code(),
            // Otherwise we print the error chain.
            None => {
                terminal::error!("{err}");
                print_error_chain(err);
                1
            }
        };

        std::process::exit(code)
    }
}

async fn _main() -> anyhow::Result<()> {
    spin_telemetry::init(VERSION.to_string()).context("Failed to initialize telemetry")?;

    let plugin_help_entries = plugin_help_entries();

    let mut cmd = SpinApp::command();
    for plugin in &plugin_help_entries {
        let subcmd = clap::Command::new(plugin.display_text())
            .about(plugin.about.as_str())
            .allow_hyphen_values(true)
            .disable_help_flag(true)
            .arg(
                clap::Arg::new("command")
                    .allow_hyphen_values(true)
                    .multiple_values(true),
            );
        cmd = cmd.subcommand(subcmd);
    }

    if !plugin_help_entries.is_empty() {
        cmd = cmd.after_help("* implemented via plugin");
    }

    let matches = cmd.clone().get_matches();

    if let Some((subcmd, _)) = matches.subcommand() {
        if plugin_help_entries.iter().any(|e| e.name == subcmd) {
            let command = std::env::args().skip(1).collect();
            return execute_external_subcommand(command, cmd).await;
        }
    }

    SpinApp::from_arg_matches(&matches)?
        .run(cmd)
        .await
        .inspect_err(|err| tracing::debug!(?err))
}

fn print_error_chain(err: anyhow::Error) {
    if let Some(cause) = err.source() {
        let is_multiple = cause.source().is_some();
        eprintln!("\nCaused by:");
        for (i, err) in err.chain().skip(1).enumerate() {
            if is_multiple {
                eprintln!("{i:>4}: {err}")
            } else {
                eprintln!("      {err}")
            }
        }
    }
}

lazy_static! {
    pub static ref VERSION: String = build_info();
}

/// Helper for passing VERSION to structopt.
fn version() -> &'static str {
    &VERSION
}

/// The Spin CLI
#[derive(Parser)]
#[clap(
    name = "spin",
    version = version()
)]
#[expect(clippy::large_enum_variant)]
enum SpinApp {
    #[clap(subcommand, alias = "template")]
    Templates(TemplateCommands),
    #[clap(alias = "n")]
    New(NewCommand),
    #[clap(alias = "a")]
    Add(AddCommand),
    #[clap(alias = "u")]
    Up(UpCommand),
    // acts as a cross-level subcommand shortcut -> `spin cloud deploy`
    #[clap(alias = "d")]
    Deploy(DeployCommand),
    // acts as a cross-level subcommand shortcut -> `spin cloud login`
    Login(LoginCommand),
    #[clap(subcommand, alias = "oci")]
    Registry(RegistryCommands),
    #[clap(alias = "b")]
    Build(BuildCommand),
    #[clap(subcommand, alias = "plugin")]
    Plugins(PluginCommands),
    #[clap(subcommand, hide = true)]
    Trigger(TriggerCommands),
    #[clap(external_subcommand)]
    External(Vec<String>),
    #[clap(alias = "w")]
    Watch(WatchCommand),
    Doctor(DoctorCommand),
    #[clap(subcommand, hide = true)]
    Maintenance(MaintenanceCommands),
}

#[derive(Subcommand)]
#[expect(clippy::large_enum_variant)]
enum TriggerCommands {
    Http(FactorsTriggerCommand<HttpTrigger, FactorsBuilder>),
    Redis(FactorsTriggerCommand<RedisTrigger, FactorsBuilder>),
    #[clap(name = spin_cli::HELP_ARGS_ONLY_TRIGGER_TYPE, hide = true)]
    HelpArgsOnly(FactorsTriggerCommand<HelpArgsOnlyTrigger, FactorsBuilder>),
}

impl SpinApp {
    /// The main entry point to Spin.
    pub async fn run(self, app: clap::Command<'_>) -> Result<(), Error> {
        match self {
            Self::Templates(cmd) => cmd.run().await,
            Self::Up(cmd) => cmd.run().await,
            Self::New(cmd) => cmd.run().await,
            Self::Add(cmd) => cmd.run().await,
            Self::Deploy(cmd) => cmd.run(SpinApp::command()).await,
            Self::Login(cmd) => cmd.run(SpinApp::command()).await,
            Self::Registry(cmd) => cmd.run().await,
            Self::Build(cmd) => cmd.run().await,
            Self::Trigger(TriggerCommands::Http(cmd)) => cmd.run().await,
            Self::Trigger(TriggerCommands::Redis(cmd)) => cmd.run().await,
            Self::Trigger(TriggerCommands::HelpArgsOnly(cmd)) => cmd.run().await,
            Self::Plugins(cmd) => cmd.run().await,
            Self::External(cmd) => execute_external_subcommand(cmd, app).await,
            Self::Watch(cmd) => cmd.run().await,
            Self::Doctor(cmd) => cmd.run().await,
            Self::Maintenance(cmd) => cmd.run(SpinApp::command()).await,
        }
    }
}

/// Returns build information, similar to: 0.1.0 (2be4034 2022-03-31).
fn build_info() -> String {
    format!("{SPIN_VERSION} ({SPIN_COMMIT_SHA} {SPIN_COMMIT_DATE})")
}

struct PluginHelpEntry {
    name: String,
    about: String,
}

impl PluginHelpEntry {
    fn from_plugin(plugin: &spin_plugins::manifest::PluginManifest) -> Option<Self> {
        if hide_plugin_in_help(plugin) {
            None
        } else {
            Some(Self {
                name: plugin.name(),
                about: plugin.description().unwrap_or_default().to_owned(),
            })
        }
    }

    fn display_text(&self) -> String {
        format!("{}*", self.name)
    }
}

fn plugin_help_entries() -> Vec<PluginHelpEntry> {
    let mut entries = installed_plugin_help_entries();
    for (name, about) in predefined_externals() {
        if !entries.iter().any(|e| e.name == name) {
            entries.push(PluginHelpEntry { name, about });
        }
    }
    entries
}

fn installed_plugin_help_entries() -> Vec<PluginHelpEntry> {
    let Ok(manager) = spin_plugins::manager::PluginManager::try_default() else {
        return vec![];
    };
    let Ok(manifests) = manager.store().installed_manifests() else {
        return vec![];
    };

    manifests
        .iter()
        .filter_map(PluginHelpEntry::from_plugin)
        .collect()
}

fn hide_plugin_in_help(plugin: &spin_plugins::manifest::PluginManifest) -> bool {
    plugin.name().starts_with("trigger-")
}
