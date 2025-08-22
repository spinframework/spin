use std::{collections::HashSet, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::Table;
use path_absolutize::Absolutize;

use serde::Serialize;
use spin_templates::{
    InstallOptions, InstallationResults, InstalledTemplateWarning, ListResults, ProgressReporter,
    SkippedReason, Template, TemplateManager, TemplateSource,
};

use crate::build_info::*;

const INSTALL_FROM_DIR_OPT: &str = "FROM_DIR";
const INSTALL_FROM_GIT_OPT: &str = "FROM_GIT";
const INSTALL_FROM_TAR_OPT: &str = "FROM_TAR";
const UPGRADE_ONLY: &str = "GIT_URL";

const DEFAULT_TEMPLATES_INSTALL_PROMPT: &str =
    "You don't have any templates yet. Would you like to install the default set?";
const DEFAULT_TEMPLATE_REPO: &str = "https://github.com/spinframework/spin";

/// Commands for working with WebAssembly component templates.
#[derive(Subcommand, Debug)]
pub enum TemplateCommands {
    /// Install templates from a Git repository or local directory.
    ///
    /// The files of the templates are copied to the local template store: a
    /// directory in your data or home directory.
    Install(Install),

    /// Upgrade templates to match your current version of Spin.
    ///
    /// The files of the templates are copied to the local template store: a
    /// directory in your data or home directory.
    Upgrade(Upgrade),

    /// Remove a template from your installation.
    Uninstall(Uninstall),

    /// List the installed templates.
    List(List),
}

impl TemplateCommands {
    pub async fn run(self) -> Result<()> {
        match self {
            TemplateCommands::Install(cmd) => cmd.run().await,
            TemplateCommands::Upgrade(cmd) => cmd.run().await,
            TemplateCommands::Uninstall(cmd) => cmd.run().await,
            TemplateCommands::List(cmd) => cmd.run().await,
        }
    }
}

/// Install templates from a Git repository or local directory.
#[derive(Parser, Debug)]
pub struct Install {
    /// The URL of the templates git repository.
    /// The templates must be in a git repository in a "templates" directory.
    #[clap(
        name = INSTALL_FROM_GIT_OPT,
        long = "git",
        alias = "repo",
        conflicts_with = INSTALL_FROM_DIR_OPT,
        conflicts_with = INSTALL_FROM_TAR_OPT,
    )]
    pub git: Option<String>,

    /// The optional branch of the git repository.
    #[clap(long = "branch", requires = INSTALL_FROM_GIT_OPT)]
    pub branch: Option<String>,

    /// Local directory containing the template(s) to install.
    #[clap(
        name = INSTALL_FROM_DIR_OPT,
        long = "dir",
        conflicts_with = INSTALL_FROM_GIT_OPT,
        conflicts_with = INSTALL_FROM_TAR_OPT,
    )]
    pub dir: Option<PathBuf>,

    /// URL to a tarball in .tar.gz format containing the template(s) to install.
    #[clap(
        name = INSTALL_FROM_TAR_OPT,
        long = "tar",
        conflicts_with = INSTALL_FROM_GIT_OPT,
        conflicts_with = INSTALL_FROM_DIR_OPT,
    )]
    pub tar_url: Option<String>,

    /// If present, updates existing templates instead of skipping.
    #[clap(long = "upgrade", alias = "update")]
    pub update: bool,
}

/// Upgrade existing template repositories from their source.
#[derive(Parser, Debug)]
pub struct Upgrade {
    /// By default, Spin displays the list of installed repositories and
    /// prompts you to choose which to upgrade.  Pass this flag to
    /// upgrade only the specified repository without prompting.
    #[clap(
        name = UPGRADE_ONLY,
        long = "repo",
    )]
    pub git: Option<String>,

    /// The optional branch of the git repository, if a specific
    /// repository is given.
    #[clap(long = "branch", requires = UPGRADE_ONLY)]
    pub branch: Option<String>,

    /// By default, Spin displays the list of installed repositories and
    /// prompts you to choose which to upgrade.  Pass this flag to
    /// upgrade all repositories without prompting.
    #[clap(long = "all", conflicts_with = UPGRADE_ONLY)]
    pub all: bool,
}

/// Remove a template from your installation.
#[derive(Parser, Debug)]
pub struct Uninstall {
    /// The template to uninstall.
    pub template_id: String,
}

impl Install {
    pub async fn run(self) -> Result<()> {
        let template_manager = TemplateManager::try_default()
            .context("Failed to construct template directory path")?;
        let source = match (&self.git, &self.dir, &self.tar_url) {
            (Some(git), None, None) => {
                let git_url = infer_github(git);
                TemplateSource::try_from_git(git_url, &self.branch, SPIN_VERSION)?
            }
            (None, Some(dir), None) => {
                let abs_dir = dir.absolutize().map(|d| d.to_path_buf());
                TemplateSource::File(abs_dir.unwrap_or_else(|_| dir.clone()))
            }
            (None, None, Some(tar_url)) => {
                let url = url::Url::parse(tar_url).context("Invalid URL for remote tar")?;
                TemplateSource::RemoteTar(url)
            }
            _ => anyhow::bail!("Exactly one of `git`, `dir`, or `tar` must be specified"),
        };

        let reporter = ConsoleProgressReporter;
        let options = InstallOptions::default().update(self.update);

        let installation_results = template_manager
            .install(&source, &options, &reporter)
            .await
            .context("Failed to install one or more templates")?;

        self.print_installed_templates(&installation_results);

        Ok(())
    }

    fn print_installed_templates(&self, installation_results: &InstallationResults) {
        let templates = &installation_results.installed;
        let skipped = &installation_results.skipped;

        if templates.is_empty() && skipped.is_empty() {
            println!("The specified source contained no templates");
        } else {
            println!("Installed {} template(s)", templates.len());
            if !templates.is_empty() {
                let mut table = Table::new();
                table.set_header(vec!["Name", "Description"]);
                table.load_preset(comfy_table::presets::ASCII_BORDERS_ONLY_CONDENSED);

                for template in templates {
                    table.add_row(vec![template.id(), template.description_or_empty()]);
                }

                println!();
                println!("{table}");
            }
            if !skipped.is_empty() {
                println!();
                println!("Skipped {} template(s)", skipped.len());

                let mut table = Table::new();
                table.set_header(vec!["Name", "Reason skipped"]);
                table.load_preset(comfy_table::presets::ASCII_BORDERS_ONLY_CONDENSED);

                for (id, reason) in skipped {
                    table.add_row(vec![id.clone(), skipped_reason_text(reason)]);
                }

                println!();
                println!("{table}");
            }
        }
    }
}

fn infer_github(raw: &str) -> String {
    match url::Url::parse(raw) {
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            if raw.starts_with('/') {
                format!("https://github.com{raw}")
            } else {
                format!("https://github.com/{raw}")
            }
        }
        _ => raw.to_string(), // pass it through even if error, so Git can have a try
    }
}

impl Upgrade {
    pub async fn run(&self) -> Result<()> {
        if self.git.is_some() {
            // This is equivalent to `install --update`
            let install = Install {
                git: self.git.clone(),
                branch: self.branch.clone(),
                dir: None,
                tar_url: None,
                update: true,
            };

            install.run().await
        } else {
            let template_manager = TemplateManager::try_default()?;
            let reporter = ConsoleProgressReporter;
            let options = InstallOptions::default().update(true);

            let selected_sources = match self.repos_to_upgrade(&template_manager).await? {
                Some(sources) => sources,
                None => return Ok(()),
            };

            let mut summary = UpgradeSummary::new();

            for source in selected_sources {
                println!("Upgrading templates from {}...", source.repo);

                let installation_results = template_manager
                    .install(&source.template_source, &options, &reporter)
                    .await;

                summary.extend_with(&source.repo, installation_results);

                println!();
            }

            self.print_upgrade_summary(&summary);

            Ok(())
        }
    }

    async fn repos_to_upgrade(
        &self,
        template_manager: &TemplateManager,
    ) -> anyhow::Result<Option<Vec<RepoSelection>>> {
        let existing_templates = template_manager.list().await?.templates;
        let (origin, no_origin): (Vec<_>, Vec<_>) = existing_templates
            .iter()
            .partition(|t| t.source_repo().is_some());

        let mut repos = origin
            .iter()
            .filter_map(|t| t.source_repo())
            .collect::<HashSet<_>>();

        // Try to detect two repos that are likely to have been installed before
        // we started recording upgrade info.
        let has_unorigined_default_templates = no_origin.iter().any(|t| t.id() == "http-rust");
        let has_unorigined_js_templates = no_origin.iter().any(|t| t.id() == "http-js");
        if has_unorigined_default_templates {
            repos.insert("https://github.com/spinframework/spin");
        }
        if has_unorigined_js_templates {
            repos.insert("https://github.com/spinframework/spin-js-sdk");
        }

        let mut sources = vec![];
        for repo in repos {
            if let Some(source) = RepoSelection::from_repo(repo).await {
                sources.push(source);
            }
        }

        if sources.is_empty() {
            eprintln!("No template repositories found to upgrade");
            eprintln!();
            if existing_templates.is_empty() {
                prompt_install_default_templates(template_manager).await?;
            } else {
                eprintln!("Your template repositories were either:");
                eprintln!("* Installed from a directory; or");
                eprintln!("* Installed using an older version of Spin");
                eprintln!("To upgrade them, run `spin templates install --upgrade` with the --git or --dir option");
            }
            return Ok(None);
        }

        // The logic here is that if there are unorigined templates, then *probably*
        // they are the most popular ones, which we will sub in anyway.  So we only
        // warn if we've already established that there are unorigined templates and
        // the most popular ones are *not* among them.  This will result in a missed
        // warning if the user has other unorigined templates too, but that's probably
        // better than printing over a dozen templates saying we won't upgrade them
        // and then offering to upgrade them all the same!  (The trouble being that we
        // can't readily exclude all the popular templates from printing without building in a
        // LOT of knowledge here. Or looking in the Git repos, or something.  But I think
        // the heuristic will cover 99% of cases.)
        let can_offer_all =
            no_origin.is_empty() || has_unorigined_default_templates || has_unorigined_js_templates;
        if !can_offer_all {
            eprintln!(
                "Spin could not determine where the following templates were installed from:"
            );
            for template in no_origin {
                eprintln!("- {}", template.id());
            }
            eprintln!("To upgrade them, run `spin templates install --upgrade` with the --git or --dir option");
            eprintln!();
            if !self.all {
                eprintln!("The following template repositories can be automatically upgraded.");
            }
        }

        let selected_sources = if self.all {
            sources
        } else {
            eprintln!("Select repos to upgrade. Use Space to select/deselect and Enter to confirm selection.");
            let selected_indexes = match dialoguer::MultiSelect::new()
                .items(&sources)
                .interact_opt()?
            {
                Some(indexes) => indexes,
                None => return Ok(None),
            };
            elements_at(sources, selected_indexes)
        };

        if selected_sources.is_empty() {
            eprintln!("No template repositories selected");
            return Ok(None);
        }
        Ok(Some(selected_sources))
    }

    fn print_upgrade_summary(&self, summary: &UpgradeSummary) {
        let templates = &summary.upgraded;
        let errors = &summary.errored_repos;

        if templates.is_empty() {
            println!("No templates were installed");
        } else {
            println!("Upgraded {} template(s)", templates.len());

            let mut table = Table::new();
            table.set_header(vec!["Name", "Description"]);
            table.load_preset(comfy_table::presets::ASCII_BORDERS_ONLY_CONDENSED);

            for template in templates {
                table.add_row(vec![template.id(), template.description_or_empty()]);
            }

            println!();
            println!("{table}");
        }

        println!();

        if !errors.is_empty() {
            // Thanks English
            println!("Errors upgrading {} repository/ies", errors.len());

            let mut table = Table::new();
            table.set_header(vec!["URL", "Error"]);
            table.load_preset(comfy_table::presets::ASCII_BORDERS_ONLY_CONDENSED);

            for (url, error) in errors {
                table.add_row(vec![url, error]);
            }

            println!();
            println!("{table}");
            println!();
        }
    }
}

struct RepoSelection {
    repo: String,
    template_source: TemplateSource,
    resolved_tag: Option<String>,
}

impl RepoSelection {
    async fn from_repo(repo: &str) -> Option<Self> {
        let template_source = TemplateSource::try_from_git(repo, &None, SPIN_VERSION).ok()?;
        let resolved_tag = template_source.resolved_tag().await;
        Some(Self {
            repo: repo.to_owned(),
            template_source,
            resolved_tag,
        })
    }
}

impl std::fmt::Display for RepoSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.repo))?;
        if let Some(tag) = &self.resolved_tag {
            f.write_fmt(format_args!(" (at {tag})"))?;
        };
        Ok(())
    }
}

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

struct UpgradeSummary {
    upgraded: Vec<Template>,
    errored_repos: Vec<(String, String)>,
}

impl UpgradeSummary {
    fn new() -> Self {
        Self {
            upgraded: vec![],
            errored_repos: vec![],
        }
    }

    fn extend_with(
        &mut self,
        url: &str,
        installation_results: anyhow::Result<InstallationResults>,
    ) {
        match installation_results {
            Ok(list) => self.upgraded.extend(list.installed),
            Err(e) => self.errored_repos.push((url.to_owned(), e.to_string())),
        }
    }
}

impl Uninstall {
    pub async fn run(self) -> Result<()> {
        let template_manager = TemplateManager::try_default()
            .context("Failed to construct template directory path")?;

        template_manager
            .uninstall(&self.template_id)
            .await
            .context("Failed to uninstall template")?;

        Ok(())
    }
}

/// List the installed templates.
#[derive(Parser, Debug)]
pub struct List {
    /// Filter templates matching all provided tags.
    #[clap(long = "tag")]
    pub tags: Vec<String>,

    /// The format in which to list the templates.
    #[clap(value_enum, long = "format", default_value = "table", hide = true)]
    pub format: ListFormat,

    /// Whether to show additional template details in the list.
    #[clap(long = "verbose", takes_value = false)]
    pub verbose: bool,
}

#[derive(ValueEnum, Clone, Debug)]
pub enum ListFormat {
    Table,
    Json,
}

impl List {
    pub async fn run(self) -> Result<()> {
        let template_manager = TemplateManager::try_default()
            .context("Failed to construct template directory path")?;

        let list_results = template_manager
            .list_with_tags(&self.tags)
            .await
            .context("Failed to list templates")?;

        match self.format {
            ListFormat::Table if list_results.needs_install() => {
                prompt_install_default_templates(&template_manager).await?;
            }
            ListFormat::Table => self.print_templates_table(&list_results),
            ListFormat::Json => self.print_templates_json(&list_results)?,
        };

        Ok(())
    }

    fn print_templates_table(&self, list_results: &ListResults) {
        let ListResults {
            templates,
            warnings,
            skipped,
        } = list_results;

        if templates.is_empty() {
            if skipped.is_empty() {
                println!();
            } else {
                let num_skipped = skipped.len();
                println!("No templates matched tags ({num_skipped} templates skipped)");
            }
        } else {
            let mut table = Table::new();

            let mut header = vec!["Name", "Description"];
            if self.verbose {
                header.push("Installed from");
            }

            table.set_header(header);
            table.load_preset(comfy_table::presets::ASCII_BORDERS_ONLY_CONDENSED);

            for template in templates {
                let mut row = vec![template.id(), template.description_or_empty()];
                if self.verbose {
                    row.push(template.installed_from_or_empty());
                }
                table.add_row(row);
            }

            println!("{table}");
        }

        if !warnings.is_empty() {
            if !templates.is_empty() {
                println!();
            }

            for (id, warning) in warnings {
                println!(
                    "note: ignored invalid entry {} ({})",
                    id,
                    list_warn_reason_text(warning)
                );
            }
        }
    }

    fn print_templates_json(&self, list_results: &ListResults) -> anyhow::Result<()> {
        let json_vals: Vec<_> = list_results
            .templates
            .iter()
            .map(json_list_format)
            .collect();

        let json_text = serde_json::to_string_pretty(&json_vals)?;
        println!("{json_text}");
        Ok(())
    }
}

fn json_list_format(template: &Template) -> TemplateListJson {
    TemplateListJson {
        id: template.id().to_owned(),
        description: template.description().as_ref().cloned(),
    }
}

#[derive(Serialize)]
struct TemplateListJson {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

struct ConsoleProgressReporter;

impl ProgressReporter for ConsoleProgressReporter {
    fn report(&self, message: impl AsRef<str>) {
        println!("{}", message.as_ref());
    }
}

fn skipped_reason_text(reason: &SkippedReason) -> String {
    match reason {
        SkippedReason::AlreadyExists => "Already exists".to_owned(),
        SkippedReason::InvalidManifest(msg) => format!("Template load error: {msg}"),
    }
}

fn list_warn_reason_text(reason: &InstalledTemplateWarning) -> String {
    match reason {
        InstalledTemplateWarning::InvalidManifest(msg) => format!("Template load error: {msg}"),
    }
}

pub(crate) async fn prompt_install_default_templates(
    template_manager: &TemplateManager,
) -> anyhow::Result<Option<Vec<Template>>> {
    let should_install = dialoguer::Confirm::new()
        .with_prompt(DEFAULT_TEMPLATES_INSTALL_PROMPT)
        .default(true)
        .interact_opt()?;
    if should_install == Some(true) {
        install_default_templates().await?;
        Ok(Some(template_manager.list().await?.templates))
    } else {
        println!(
            "You can install the default templates later with 'spin templates install --git {DEFAULT_TEMPLATE_REPO}'"
        );
        Ok(None)
    }
}

async fn install_default_templates() -> anyhow::Result<()> {
    let install_cmd = Install {
        git: Some(DEFAULT_TEMPLATE_REPO.to_owned()),
        branch: None,
        dir: None,
        tar_url: None,
        update: false,
    };
    install_cmd
        .run()
        .await
        .context("Failed to install the default templates")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_github_url_if_needed() {
        assert_eq!(
            "https://github.com/spinframework/spin",
            infer_github("spinframework/spin")
        );
        assert_eq!(
            "https://github.com/spinframework/spin",
            infer_github("/spinframework/spin")
        );
        assert_eq!(
            "https://github.com/spinframework/spin",
            infer_github("https://github.com/spinframework/spin")
        );
    }
}
