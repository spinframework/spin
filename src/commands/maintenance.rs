use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Commands for Spin maintenance tasks.
#[derive(Subcommand, Debug)]
pub enum MaintenanceCommands {
    /// Generate CLI reference docs in Markdown format.
    GenerateReference(GenerateReference),
    /// Generate JSON schema for application manifest.
    GenerateManifestSchema(GenerateSchema),
    /// Generate a `completely` file which can then be processed into shell completions.
    GenerateShellCompletions(GenerateCompletions),
}

impl MaintenanceCommands {
    pub async fn run(&self, app: clap::Command<'_>) -> anyhow::Result<()> {
        match self {
            MaintenanceCommands::GenerateReference(cmd) => cmd.run(app).await,
            MaintenanceCommands::GenerateManifestSchema(cmd) => cmd.run().await,
            MaintenanceCommands::GenerateShellCompletions(cmd) => cmd.run(app).await,
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
    pub async fn run(&self, app: clap::Command<'_>) -> anyhow::Result<()> {
        let markdown = crate::clap_markdown::help_markdown_command(&app);
        write(&self.output, &markdown)?;
        Ok(())
    }
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
pub struct GenerateCompletions {
    /// The file to which to generate the completions. If omitted, it is generated to stdout.
    #[clap(short = 'o')]
    pub output: Option<PathBuf>,
}

impl GenerateCompletions {
    async fn run(&self, cmd: clap::Command<'_>) -> anyhow::Result<()> {
        let writer: &mut dyn std::io::Write = match &self.output {
            None => &mut std::io::stdout(),
            Some(path) => &mut std::fs::File::create(path).unwrap(),
        };

        generate_completely_yaml(&cmd, writer);

        Ok(())
    }
}

fn generate_completely_yaml(cmd: &clap::Command, buf: &mut dyn std::io::Write) {
    let mut completion_map = serde_json::value::Map::new();

    let subcommands = visible_subcommands(cmd);

    insert_array(
        &mut completion_map,
        cmd.get_name(),
        subcommands.iter().map(|sc| sc.get_name()),
    );

    for subcmd in subcommands {
        append_subcommand(&mut completion_map, subcmd, &format!("{} ", cmd.get_name()));
    }

    let j = serde_json::Value::Object(completion_map);
    serde_json::to_writer_pretty(buf, &j).unwrap();
}

fn append_subcommand(
    completion_map: &mut serde_json::value::Map<String, serde_json::Value>,
    subcmd: &clap::Command<'_>,
    prefix: &str,
) {
    let key = format!("{}{}", prefix, subcmd.get_name());

    let subsubcmds = visible_subcommands(subcmd);

    let positionals = subcmd
        .get_arguments()
        .filter(|a| a.is_positional())
        .map(|a| hint(&key, a).to_owned())
        .filter(|h| !h.is_empty());
    let subsubcmd_names = subsubcmds.iter().map(|c| c.get_name().to_owned());
    let flags = subcmd
        .get_arguments()
        .filter(|a| !a.is_hide_set())
        .flat_map(long_and_short);
    let subcmd_options = positionals.chain(subsubcmd_names).chain(flags);

    insert_array(completion_map, &key, subcmd_options);

    for arg in subcmd.get_arguments() {
        // We have already done positionals - this is for `cmd*--flag` arrays
        if arg.is_positional() || !arg.is_takes_value_set() {
            continue;
        }

        let hint = hint(&key, arg);
        for flag in long_and_short(arg) {
            let key = format!("{key}*{flag}");
            insert_array(completion_map, &key, std::iter::once(hint));
        }
    }

    for subsubcmd in &subsubcmds {
        append_subcommand(completion_map, subsubcmd, &format!("{key} "));
    }
}

fn hint(full_cmd: &str, arg: &clap::Arg<'_>) -> &'static str {
    match arg.get_value_hint() {
        clap::ValueHint::AnyPath => "<file>",
        clap::ValueHint::FilePath => "<file>",
        clap::ValueHint::DirPath => "<directory>",
        _ => custom_hint(full_cmd, arg),
    }
}

fn custom_hint(full_cmd: &str, arg: &clap::Arg<'_>) -> &'static str {
    let arg_name = arg.get_long();

    match (full_cmd, arg_name) {
        // ("spin build", Some("component-id")) - no existing cmd. We'd ideally want a way to infer app path too
        ("spin new", Some("template")) => "$(spin templates list --format names-only 2>/dev/null)",
        ("spin plugins uninstall", None) => {
            "$(spin plugins list --installed --format names-only 2>/dev/null)"
        }
        ("spin plugins upgrade", None) => {
            "$(spin plugins list --installed --format names-only 2>/dev/null)"
        }
        ("spin templates uninstall", None) => {
            "$(spin templates list --format names-only 2>/dev/null)"
        }
        // ("spin up", Some("component-id")) - no existing cmd. We'd ideally want a way to infer app path too
        _ => "",
    }
}

fn visible_subcommands<'a, 'b>(cmd: &'a clap::Command<'b>) -> Vec<&'a clap::Command<'b>> {
    cmd.get_subcommands()
        .filter(|sc| !sc.is_hide_set())
        .collect()
}

fn insert_array<T: Into<String>>(
    map: &mut serde_json::value::Map<String, serde_json::Value>,
    key: impl Into<String>,
    values: impl Iterator<Item = T>,
) {
    let key = key.into();
    let values = values
        .map(|s| serde_json::Value::String(s.into()))
        .collect();
    map.insert(key, values);
}

fn long_and_short(arg: &clap::Arg<'_>) -> Vec<String> {
    let mut result = vec![];
    if let Some(c) = arg.get_short() {
        result.push(format!("-{c}"));
    }
    if let Some(s) = arg.get_long() {
        result.push(format!("--{s}"));
    }
    result
}
