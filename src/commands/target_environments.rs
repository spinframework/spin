use clap::Subcommand;
use semver::Version;
use std::cmp::Ordering;

/// Commands for the target environments catalogue.
#[derive(Subcommand, Debug)]
pub enum TargetEnvironmentCommands {
    /// List known target environments.
    List,

    /// Update the target environments from the remote repository.
    Update,
}

impl TargetEnvironmentCommands {
    pub async fn run(self) -> anyhow::Result<()> {
        match self {
            Self::List => list().await,
            Self::Update => update().await,
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
