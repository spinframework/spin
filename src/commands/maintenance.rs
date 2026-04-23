use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};

/// Commands for Spin maintenance tasks.
#[derive(Subcommand, Debug)]
pub enum MaintenanceCommands {
    /// Generate CLI reference docs in Markdown format.
    GenerateReference(GenerateReference),
    /// Generate JSON schema for application manifest.
    GenerateManifestSchema(GenerateSchema),
    /// Generate shell completions. Requires the COMPLETE environment variable to be set.
    GenerateCompletions,
}

impl MaintenanceCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            MaintenanceCommands::GenerateReference(cmd) => cmd.run().await,
            MaintenanceCommands::GenerateManifestSchema(cmd) => cmd.run().await,
            MaintenanceCommands::GenerateCompletions => GenerateCompletions::run().await,
        }
    }
}

#[derive(Parser, Debug)]
pub struct GenerateReference {
    /// The file to which to generate the reference Markdown. If omitted, it is generated to stdout.
    #[clap(short = 'o')]
    pub output: Option<PathBuf>,
}

impl GenerateReference {
    pub async fn run(&self) -> anyhow::Result<()> {
        let cmd = sorted_command(&crate::SpinApp::command());
        let markdown = clap_markdown::help_markdown_command_custom(
            &cmd,
            &clap_markdown::MarkdownOptions::new().show_aliases(false),
        );
        write(&self.output, &markdown)?;
        Ok(())
    }
}

/// Rebuild a `clap::Command` with subcommands and options sorted alphabetically.
/// This preserves the sorted output from the previously vendored clap-markdown fork.
fn sorted_command(cmd: &clap::Command) -> clap::Command {
    if cmd.get_name() == "up" {
        let inner = crate::commands::up::UpCommand::inner()
            // We have to munge the name to stop it recursing.
            .name("up-inner");
        return sorted_command(&inner);
    }

    let mut new_cmd = clap::Command::new(cmd.get_name().to_owned());

    // Because of the `up` shenanigans, we have to remove the help and
    // version flags or Clap asserts on a duplicate flag in `watch`.
    // (But this is no loss because clap-markdown skips them anyway.)
    new_cmd = new_cmd.disable_help_flag(true).disable_version_flag(true);

    // Unmunge the name munging
    if cmd.get_name() == "up-inner" {
        new_cmd = new_cmd.name("up");
    }

    if let Some(v) = cmd.get_display_name() {
        new_cmd = new_cmd.display_name(v);
    }
    if let Some(v) = cmd.get_bin_name() {
        new_cmd = new_cmd.bin_name(v);
    }
    if let Some(v) = cmd.get_about() {
        new_cmd = new_cmd.about(v.to_owned());
    }
    if let Some(v) = cmd.get_long_about() {
        new_cmd = new_cmd.long_about(v.to_owned());
    }
    if let Some(v) = cmd.get_before_help() {
        new_cmd = new_cmd.before_help(v.to_owned());
    }
    if let Some(v) = cmd.get_before_long_help() {
        new_cmd = new_cmd.before_long_help(v.to_owned());
    }
    if let Some(v) = cmd.get_after_help() {
        new_cmd = new_cmd.after_help(v.to_owned());
    }
    if let Some(v) = cmd.get_after_long_help() {
        new_cmd = new_cmd.after_long_help(v.to_owned());
    }
    new_cmd = new_cmd.hide(cmd.is_hide_set());
    new_cmd = new_cmd.subcommand_required(cmd.is_subcommand_required_set());

    // Copy positional arguments (preserve definition order)
    for arg in cmd.get_positionals() {
        new_cmd = new_cmd.arg(arg.clone());
    }

    // Copy non-positional arguments sorted by short flag / long name
    let mut non_pos: Vec<_> = cmd
        .get_arguments()
        .filter(|a| !a.is_positional())
        .cloned()
        .collect();
    non_pos.sort_by_key(|arg| {
        arg.get_short()
            .map(|c| c.to_string())
            .or_else(|| arg.get_long().map(|l| l.to_string()))
            .unwrap_or_else(|| "zzz".to_string())
    });
    for arg in non_pos {
        new_cmd = new_cmd.arg(arg);
    }

    // Copy argument groups
    for group in cmd.get_groups() {
        new_cmd = new_cmd.group(group.clone());
    }

    // Add subcommands sorted alphabetically (recursively)
    let mut subs: Vec<clap::Command> = cmd.get_subcommands().cloned().collect();
    subs.sort_by_key(|c| c.get_name().to_owned());
    for sub in subs {
        new_cmd = new_cmd.subcommand(sorted_command(&sub));
    }

    new_cmd
}

#[derive(Parser, Debug)]
pub struct GenerateSchema {
    /// The file to which to generate the JSON schema. If omitted, it is generated to stdout.
    #[clap(short = 'o')]
    pub output: Option<PathBuf>,
}

impl GenerateSchema {
    async fn run(&self) -> anyhow::Result<()> {
        let schema = schemars::schema_for!(spin_manifest::schema::v2::AppManifest);
        let schema_json = serde_json::to_string_pretty(&schema)?;
        write(&self.output, &schema_json)?;
        Ok(())
    }
}

fn write(output: &Option<PathBuf>, text: &str) -> anyhow::Result<()> {
    match output {
        Some(path) => std::fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}

#[derive(Parser, Debug)]
pub struct GenerateCompletions;

impl GenerateCompletions {
    async fn run() -> anyhow::Result<()> {
        // This intentionally does not use `crate::is_completions_request`.
        // The reason is that `is_completions_request` may grow stronger, because it must
        // avoid inferring a completion scenario when there isn't one (because the
        // consequence of getting it wrong would be 'you can't run Spin commands').
        // Whereas this check has to be as weak as possible, because it must avoid inferring a
        // *non*-completion scenario when there *is* one (because the consequence
        // of doing so would be 'you can't generate completions', whereas the consequence of
        // failing to spot a non-completion scenario is merely 'you don't get a good error').
        if std::env::var_os("COMPLETE").is_none() {
            anyhow::bail!(
                "Set the COMPLETE environment variable to the name of your shell while generating completions."
            );
        }

        // `SpinApp::Up` is associated with `UpCommand`, which does its work by clever
        // parsing tricks. `clap_complete` is frustratingly oblivious to these clever
        // parsing tricks, so if you run it over `SpinApp`, it doesn't generate completions
        // for `spin up`. So we sub in the `inner()` which carries the actual 'real' clap parsing.
        let factory = || {
            let cmd = crate::SpinApp::command();
            let up_inner = crate::commands::up::UpCommand::inner()
                .name("up")
                .alias("u");
            cmd.mut_subcommand("up", |_| up_inner)
        };

        let env = clap_complete::env::CompleteEnv::with_factory(factory);
        env.complete();

        Ok(())
    }
}
