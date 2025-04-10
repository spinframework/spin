mod store;

use std::io::IsTerminal;

use self::store::{BadgerRecordManager, PreviousBadger};
use crate::manifest::PluginManifest;

const BADGER_TIMEOUT_DAYS: i64 = 14;

// How the checker works:
//
// * The consumer calls BadgerChecker::start().  This immediately returns a task handle to
//   the checker.  It's important that this be immediate, because it's called on _every_
//   plugin invocation and we don't want to slow that down.
// * In the background task, the checker determines if it needs to update the local copy
//   of the plugins registry.  If so, it kicks that off as a background process.
//   * The checker may determine while running the task that the user should not be prompted,
//     or hit an error trying to kick things off the check.  In this case, it returns
//     BadgerChecker::Precomputed from the task, ready to be picked up.
//   * Otherwise, the checker wants to wait as long as possible before determining whether
//     an upgrade is possible.  In this case it returns BadgerChecker::Deferred from the task.
//     This captures the information needed for the upgrade check.
// * When the consumer is ready to find out if it needs to notify the user, it awaits
//   the task handle.  This should still be quick.
// * The consumer then calls BadgerChecker::check().
//   * If the task returned Precomputed (i.e. the task reached a decision before exiting),
//     check() returns that precomputed value.
//   * If the task returned Deferred (i.e. the task was holding off to let the background registry
//     update do its work), it now loads the local copy of the registry, and compares the
//     available versions to the current version.
//
// The reason for the Precomputed/Deferred dance is to handle the two cases of:
// 1. There's no point waiting and doing the calculations because we _know_ we have a decision (or an error).
// 2. There's a point to waiting because there _might_ be an upgrade, so we want to give the background
//    process as much time as possible to complete, so we can offer the latest upgrade.

pub enum BadgerChecker {
    Precomputed(anyhow::Result<BadgerUI>),
    Deferred(BadgerEvaluator),
}

pub struct BadgerEvaluator {
    plugin_name: String,
    current_version: semver::Version,
    spin_version: &'static str,
    plugin_manager: crate::manager::PluginManager,
    record_manager: BadgerRecordManager,
    previous_badger: PreviousBadger,
}

impl BadgerChecker {
    pub fn start(
        name: &str,
        current_version: Option<String>,
        spin_version: &'static str,
    ) -> tokio::task::JoinHandle<Self> {
        let name = name.to_owned();

        tokio::task::spawn(async move {
            let current_version = match current_version {
                Some(v) => v.to_owned(),
                None => return Self::Precomputed(Ok(BadgerUI::None)),
            };

            if !std::io::stderr().is_terminal() {
                return Self::Precomputed(Ok(BadgerUI::None));
            }

            match BadgerEvaluator::new(&name, &current_version, spin_version).await {
                Ok(b) => {
                    if b.should_check() {
                        // We want to offer the user an upgrade if one is available. Kick off a
                        // background process to update the local copy of the registry, and
                        // return the case that causes Self::check() to consult the registry.
                        BadgerEvaluator::fire_and_forget_update();
                        Self::Deferred(b)
                    } else {
                        // We do not want to offer the user an upgrade, e.g. because we have
                        // badgered them quite recently. Stash this decision for Self::check()
                        // to return.
                        Self::Precomputed(Ok(BadgerUI::None))
                    }
                }
                Err(e) => {
                    // We hit a problem determining if we wanted to offer an upgrade or not.
                    // Stash the error for Self::check() to return.
                    Self::Precomputed(Err(e))
                }
            }
        })
    }

    pub async fn check(self) -> anyhow::Result<BadgerUI> {
        match self {
            Self::Precomputed(r) => r,
            Self::Deferred(b) => b.check().await,
        }
    }
}

impl BadgerEvaluator {
    async fn new(
        name: &str,
        current_version: &str,
        spin_version: &'static str,
    ) -> anyhow::Result<Self> {
        let current_version = semver::Version::parse(current_version)?;
        let plugin_manager = crate::manager::PluginManager::try_default()?;
        let record_manager = BadgerRecordManager::default()?;
        let previous_badger = record_manager.previous_badger(name, &current_version).await;

        Ok(Self {
            plugin_name: name.to_owned(),
            current_version,
            spin_version,
            plugin_manager,
            record_manager,
            previous_badger,
        })
    }

    fn should_check(&self) -> bool {
        match self.previous_badger {
            PreviousBadger::Fresh => true,
            PreviousBadger::FromCurrent { when, .. } => has_timeout_expired(when),
        }
    }

    fn fire_and_forget_update() {
        if let Err(e) = Self::fire_and_forget_update_impl() {
            tracing::info!("Failed to launch plugins update process; checking using latest local repo anyway. Error: {e:#}");
        }
    }

    fn fire_and_forget_update_impl() -> anyhow::Result<()> {
        let mut update_cmd = tokio::process::Command::new(std::env::current_exe()?);
        update_cmd.args(["plugins", "update"]);
        update_cmd.stdout(std::process::Stdio::null());
        update_cmd.stderr(std::process::Stdio::null());
        update_cmd.spawn()?;
        Ok(())
    }

    async fn check(&self) -> anyhow::Result<BadgerUI> {
        let available_upgrades = self.available_upgrades().await?;

        // TO CONSIDER: skipping this check and badgering for the same upgrade in case they missed it
        if self
            .previous_badger
            .includes_any(&available_upgrades.list())
        {
            return Ok(BadgerUI::None);
        }

        if !available_upgrades.is_none() {
            self.record_manager
                .record_badger(
                    &self.plugin_name,
                    &self.current_version,
                    &available_upgrades.list(),
                )
                .await
        };

        Ok(available_upgrades.classify())
    }

    async fn available_upgrades(&self) -> anyhow::Result<AvailableUpgrades> {
        let store = self.plugin_manager.store();

        let latest_version = {
            let latest_lookup = crate::lookup::PluginLookup::new(&self.plugin_name, None);
            let latest_manifest = latest_lookup
                .resolve_manifest_exact(store.get_plugins_directory())
                .await
                .ok();
            latest_manifest.and_then(|m| semver::Version::parse(m.version()).ok())
        };

        let manifests = store.catalogue_manifests()?;
        let relevant_manifests = manifests
            .into_iter()
            .filter(|m| m.name() == self.plugin_name);
        let compatible_manifests = relevant_manifests.filter(|m| {
            m.has_compatible_package() && m.is_compatible_spin_version(self.spin_version)
        });
        let compatible_plugin_versions =
            compatible_manifests.filter_map(|m| PluginVersion::try_from(m, &latest_version));
        let considerable_manifests = compatible_plugin_versions
            .filter(|pv| !pv.is_prerelease() && pv.is_higher_than(&self.current_version))
            .collect::<Vec<_>>();

        let (eligible_manifests, questionable_manifests) = if self.current_version.major == 0 {
            (vec![], considerable_manifests)
        } else {
            considerable_manifests
                .into_iter()
                .partition(|pv| pv.version.major == self.current_version.major)
        };

        let highest_eligible_manifest = eligible_manifests
            .into_iter()
            .max_by_key(|pv| pv.version.clone());
        let highest_questionable_manifest = questionable_manifests
            .into_iter()
            .max_by_key(|pv| pv.version.clone());

        Ok(AvailableUpgrades {
            eligible: highest_eligible_manifest,
            questionable: highest_questionable_manifest,
        })
    }
}

fn has_timeout_expired(from_time: chrono::DateTime<chrono::Utc>) -> bool {
    let timeout = chrono::Duration::try_days(BADGER_TIMEOUT_DAYS).unwrap();
    let now = chrono::Utc::now();
    match now.checked_sub_signed(timeout) {
        None => true,
        Some(t) => from_time < t,
    }
}

pub struct AvailableUpgrades {
    eligible: Option<PluginVersion>,
    questionable: Option<PluginVersion>,
}

impl AvailableUpgrades {
    fn is_none(&self) -> bool {
        self.eligible.is_none() && self.questionable.is_none()
    }

    fn classify(&self) -> BadgerUI {
        match (&self.eligible, &self.questionable) {
            (None, None) => BadgerUI::None,
            (Some(e), None) => BadgerUI::Eligible(e.clone()),
            (None, Some(q)) => BadgerUI::Questionable(q.clone()),
            (Some(e), Some(q)) => BadgerUI::Both {
                eligible: e.clone(),
                questionable: q.clone(),
            },
        }
    }

    fn list(&self) -> Vec<&semver::Version> {
        [self.eligible.as_ref(), self.questionable.as_ref()]
            .iter()
            .filter_map(|pv| pv.as_ref())
            .map(|pv| &pv.version)
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct PluginVersion {
    version: semver::Version,
    name: String,
    is_latest: bool,
}

impl PluginVersion {
    fn try_from(manifest: PluginManifest, latest: &Option<semver::Version>) -> Option<Self> {
        match semver::Version::parse(manifest.version()) {
            Ok(version) => {
                let name = manifest.name();
                let is_latest = match latest {
                    None => false,
                    Some(latest) => &version == latest,
                };
                Some(Self {
                    version,
                    name,
                    is_latest,
                })
            }
            Err(_) => None,
        }
    }

    fn is_prerelease(&self) -> bool {
        !self.version.pre.is_empty()
    }

    fn is_higher_than(&self, other: &semver::Version) -> bool {
        &self.version > other
    }

    pub fn upgrade_command(&self) -> String {
        if self.is_latest {
            format!("spin plugins upgrade {}", self.name)
        } else {
            format!("spin plugins upgrade {} -v {}", self.name, self.version)
        }
    }
}

impl std::fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.version)
    }
}

pub enum BadgerUI {
    // Do not badger the user. There is no available upgrade, or we have already badgered
    // them recently about this plugin.
    None,
    // There is an available upgrade which is compatible (same non-zero major version).
    Eligible(PluginVersion),
    // There is an available upgrade but it may not be compatible (different major version
    // or major version is zero).
    Questionable(PluginVersion),
    // There is an available upgrade which is compatible, but there is also an even more
    // recent upgrade which may not be compatible.
    Both {
        eligible: PluginVersion,
        questionable: PluginVersion,
    },
}
