use crate::commands::external::execute_external_subcommand;
use anyhow::Result;
use clap::Args;

#[derive(Debug, Args, PartialEq)]
#[clap(
    about = "Package and upload an application to a deployment environment.",
    allow_hyphen_values = true,
    disable_help_flag = true
)]
pub struct DeployCommand {
    /// All args to be passed through to the plugin
    #[clap(hide = true)]
    args: Vec<String>,
}

#[derive(Debug, Args, PartialEq)]
#[clap(
    about = "Log into a deployment environment.",
    allow_hyphen_values = true,
    disable_help_flag = true
)]
pub struct LoginCommand {
    /// All args to be passed through to the plugin
    #[clap(hide = true)]
    args: Vec<String>,
}

/// Transitional for compatibility: this will be removed as part of vendor-neutrality work.
const DEFAULT_DEPLOY_PLUGIN: &str = "cloud";

/// The environment variable for setting the plugin to be used for operations relating
/// to remote hosts. This allows the `spin deploy` and `spin login` shortcuts instead of
/// `spin whatever deploy` etc.
const DEPLOY_PLUGIN_ENV: &str = "SPIN_DEPLOY_PLUGIN";

impl DeployCommand {
    pub async fn run(self, cmd: clap::Command<'_>) -> Result<()> {
        const SUBCMD: &str = "deploy";

        let deploy_plugin = deployment_plugin(SUBCMD)?;
        let mut subcmd = vec![deploy_plugin, SUBCMD.to_string()];
        subcmd.append(&mut self.args.clone());
        execute_external_subcommand(subcmd, cmd).await
    }
}

impl LoginCommand {
    pub async fn run(self, cmd: clap::Command<'_>) -> Result<()> {
        const SUBCMD: &str = "login";

        let deploy_plugin = deployment_plugin(SUBCMD)?;
        let mut subcmd = vec![deploy_plugin, SUBCMD.to_string()];
        subcmd.append(&mut self.args.clone());
        execute_external_subcommand(subcmd, cmd).await
    }
}

fn deployment_plugin(cmd: &str) -> anyhow::Result<String> {
    match std::env::var(DEPLOY_PLUGIN_ENV) {
        Ok(v) => Ok(v),
        Err(std::env::VarError::NotPresent) => {
            terminal::warn!("`spin {cmd}` will soon need to be told which deployment plugin to use.\nRun a plugin command (e.g. `spin {DEFAULT_DEPLOY_PLUGIN} {cmd}`), or set the `{DEPLOY_PLUGIN_ENV}` environment variable, instead.\nDefaulting to `{DEFAULT_DEPLOY_PLUGIN}` plugin.");
            Ok(DEFAULT_DEPLOY_PLUGIN.to_string())
        }
        Err(_) => anyhow::bail!("{DEPLOY_PLUGIN_ENV} was defined but its value could not be read"),
    }
}
