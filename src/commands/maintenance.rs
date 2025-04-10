use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Commands for Spin maintenance tasks.
#[derive(Subcommand, Debug)]
pub enum MaintenanceCommands {
    /// Generate CLI reference docs in Markdown format.
    GenerateReference(GenerateReference),
    /// Generate JSON schema for application manifest.
    GenerateManifestSchema(GenerateSchema),
}

impl MaintenanceCommands {
    pub async fn run(&self, app: clap::Command<'_>) -> anyhow::Result<()> {
        match self {
            MaintenanceCommands::GenerateReference(cmd) => cmd.run(app).await,
            MaintenanceCommands::GenerateManifestSchema(cmd) => cmd.run().await,
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
