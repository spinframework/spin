use anyhow::Context;
use clap::{Parser, Subcommand};
use semver::Version;
use std::cmp::Ordering;
use std::path::PathBuf;

use spin_manifest::schema::v2::TargetEnvironmentRef;

use crate::opts::APP_MANIFEST_FILE_OPT;

/// Commands for the target environments catalogue.
#[derive(Subcommand, Debug)]
pub enum TargetEnvironmentCommands {
    /// List known target environments.
    List,

    /// Update the target environments from the remote repository.
    Update,

    /// Check whether an application is compatible with one or more target environments.
    Check(CheckCommand),
}

impl TargetEnvironmentCommands {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            Self::List => list().await,
            Self::Update => update().await,
            Self::Check(cmd) => check(cmd).await,
        }
    }
}

async fn list() -> anyhow::Result<()> {
    let catalogue = spin_environments::Catalogue::try_default()?;

    let mut envs = catalogue.list().await;
    envs.sort_by(|p, q| order_versioned(p, q));

    if envs.is_empty() {
        eprintln!(
            "No target environments found. Run `spin targets update` to fetch target environments."
        );
    }

    for env in &envs {
        println!("{env}");
    }

    Ok(())
}

async fn update() -> anyhow::Result<()> {
    let catalogue = spin_environments::Catalogue::try_default()?;

    catalogue.update().await?;

    eprintln!("Target environments updated");
    Ok(())
}

/// Check whether an application is compatible with one or more target environments.
#[derive(Parser, Debug)]
pub struct CheckCommand {
    /// The application to check. This may be a manifest (spin.toml) file, or a
    /// directory containing a spin.toml file. If omitted, it defaults to "spin.toml".
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
    )]
    pub app_source: Option<PathBuf>,

    /// The target environment(s) to check against. This may be a catalogue id (for
    /// example `spin-up@3.6`), an `http:` URL, or a `file:` path, and may be specified
    /// multiple times. If omitted, the targets declared in the application manifest
    /// are checked.
    #[clap(short = 'E', long = "target")]
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::environments))]
    pub target_environment: Vec<String>,

    /// The build profile to use when building the application before checking.
    #[clap(long)]
    #[arg(add = clap_complete::ArgValueCandidates::new(crate::completions::profiles))]
    pub profile: Option<String>,

    /// Check the application as already built, rather than building it first.
    #[clap(long)]
    pub no_build: bool,
}

async fn check(cmd: CheckCommand) -> anyhow::Result<()> {
    let (manifest_file, distance) =
        spin_common::paths::find_manifest_file_path(cmd.app_source.as_ref())?;
    crate::directory_rels::notify_if_nondefault_rel(&manifest_file, distance);

    let app_dir = spin_common::paths::parent_dir(&manifest_file)?;
    let profile = cmd.profile.as_deref();

    // Target checking inspects each component's compiled Wasm, so the application
    // must be built first. We skip Spin's own manifest-driven target checks here,
    // because we run our own check against the requested environments below.
    if !cmd.no_build {
        spin_build::build(
            &manifest_file,
            profile,
            &[],
            spin_build::TargetChecking::Skip,
            spin_build::GenerateDependencyWits::Generate,
            None,
        )
        .await?;
    }

    let manifest = spin_manifest::manifest_from_file(&manifest_file)
        .context("Failed to read application manifest for target environment checking")?;

    // Environments named on the command line take precedence; otherwise fall back
    // to the targets declared in the manifest.
    let target_refs: Vec<TargetEnvironmentRef> = if cmd.target_environment.is_empty() {
        manifest.application.targets.clone()
    } else {
        cmd.target_environment
            .iter()
            .map(|env| crate::parse_env::parse_env(env))
            .collect()
    };

    if target_refs.is_empty() {
        anyhow::bail!(
            "No target environments to check. Specify one or more with -E (for example `-E spin-up@3.6`), or declare `targets` in the application manifest."
        );
    }

    let application =
        spin_environments::ApplicationToValidate::new(manifest, &[], profile, &app_dir)
            .await
            .context("unable to load application for checking against target environments")?;

    let targets = spin_environments::Targets {
        default: &target_refs,
        overrides: Default::default(),
    };

    let validation = spin_environments::validate_application_against_environment_ids(
        &application,
        targets,
        None,
        &app_dir,
    )
    .await
    .context("unable to check if the application is compatible with the target environments")?;

    if validation.is_ok() {
        terminal::step!(
            "Compatible",
            "the application is compatible with all checked target environments."
        );
        Ok(())
    } else {
        for error in validation.errors() {
            terminal::error!("{error}");
        }
        anyhow::bail!("The application is not compatible with one or more target environments.");
    }
}

fn order_versioned(first: &str, second: &str) -> Ordering {
    let (name1, ver1) = name_and_version(first);
    let (name2, ver2) = name_and_version(second);

    match name1.cmp(name2) {
        Ordering::Equal => match (ver1, ver2) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(ver1), Some(ver2)) => order_env_ver(ver1, ver2),
        },
        other => other,
    }
}

fn name_and_version(env_name: &str) -> (&str, Option<&str>) {
    match env_name.rsplit_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (env_name, None),
    }
}

fn order_env_ver(first: &str, second: &str) -> Ordering {
    // Environment versions are expected to be major.minor, but this allows for
    // some crazy future where a bold young experimentalist creates a major-only
    // or patch-specific env.

    // Leverage semver for parsing and ordering
    fn padded_version(v: &str) -> Option<Version> {
        let padded = match v.split('.').count() {
            1 => format!("{v}.0.0"),
            2 => format!("{v}.0"),
            _ => v.to_string(),
        };
        Version::parse(&padded).ok()
    }

    match (padded_version(first), padded_version(second)) {
        (Some(v1), Some(v2)) => v1.cmp(&v2),
        _ => first.cmp(second),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn compare_understands_versions() {
        assert_eq!(Ordering::Less, order_versioned("capybara", "cassowary"));
        assert_eq!(Ordering::Equal, order_versioned("capybara", "capybara"));
        assert_eq!(Ordering::Greater, order_versioned("capybara", "amphipod"));

        assert_eq!(
            Ordering::Less,
            order_versioned("capybara@3.2", "cassowary@1.2")
        );
        assert_eq!(
            Ordering::Equal,
            order_versioned("capybara@3.2", "capybara@3.2")
        );
        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara@3.2", "amphipod@1.2")
        );

        assert_eq!(Ordering::Less, order_versioned("capybara@3.2", "cassowary"));
        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara@3.2", "capybara")
        );
        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara@3.2", "amphipod")
        );

        assert_eq!(
            Ordering::Less,
            order_versioned("capybara@3.2", "cassowary@1.2")
        );
        assert_eq!(Ordering::Less, order_versioned("capybara", "capybara@3.2"));
        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara", "amphipod@1.2")
        );

        assert_eq!(
            Ordering::Less,
            order_versioned("capybara@1.2", "capybara@3.2")
        );
        assert_eq!(
            Ordering::Less,
            order_versioned("capybara@1.3", "capybara@1.29")
        );
        assert_eq!(
            Ordering::Less,
            order_versioned("capybara@3.2", "capybara@222.111")
        );

        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara@3.2", "capybara@1.2")
        );
        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara@1.29", "capybara@1.3")
        );
        assert_eq!(
            Ordering::Greater,
            order_versioned("capybara@222.111", "capybara@3.2")
        );
    }
}
