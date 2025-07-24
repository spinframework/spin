use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use itertools::Itertools;
use path_absolutize::Absolutize;
use spin_common::paths::parent_dir;
use uuid::Uuid;
use watchexec::Watchexec;

use crate::{
    directory_rels::notify_if_nondefault_rel,
    opts::{APP_MANIFEST_FILE_OPT, WATCH_CLEAR_OPT, WATCH_DEBOUNCE_OPT, WATCH_SKIP_BUILD_OPT},
};

mod buildifier;
mod filters;
mod reconfiguriser;
mod uppificator;

use buildifier::Buildifier;
use filters::{ArtifactFilterFactory, BuildFilterFactory, FilterFactory, ManifestFilterFactory};
use uppificator::{Pause, Uppificator};

/// Build and run the Spin application, rebuilding and restarting it when files change.
#[derive(Parser, Debug)]
#[clap(
    about = "Build and run the Spin application, rebuilding and restarting it when files change",
    allow_hyphen_values = true
)]
pub struct WatchCommand {
    /// The application to watch. This may be a manifest (spin.toml) file, or a
    /// directory containing a spin.toml file.
    /// If omitted, it defaults to "spin.toml".
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
    )]
    pub app_source: Option<PathBuf>,

    /// Clear the screen before each run.
    #[clap(
            name = WATCH_CLEAR_OPT,
            short = 'c',
            long = "clear",
    )]
    pub clear: bool,

    /// Set the timeout between detected change and re-execution, in milliseconds.
    #[clap(
        name = WATCH_DEBOUNCE_OPT,
        short = 'd',
        long = "debounce",
        default_value = "100"
    )]
    pub debounce: u64,

    /// Only run the Spin application, restarting it when build artifacts change.
    #[clap(name = WATCH_SKIP_BUILD_OPT, long = "skip-build")]
    pub skip_build: bool,

    /// Arguments to be passed through to spin up.
    #[clap()]
    pub up_args: Vec<String>,
}

impl WatchCommand {
    pub async fn run(self) -> Result<()> {
        // Strategy:
        // * The Uppificator runs `spin up`, and watches the manifest artifacts (component.source and component.files)
        //   (and the manifest if build is not in play). When it detects a change, it restarts `spin up`.
        //   THAT'S ALL, THAT'S ALL IT DOES.
        //   * If `spin up` crashes, the Uppificator restarts it.  BUT APART FROM THAT THAT'S ALL IT DOES OKAY.
        // * The Buildifier, if in play, watches the manifest and component.build.watch collections. When it detects a
        //   change, it PAUSES the Uppificator, does the build, then unpauses the Uppificator.
        //   * It is on the Uppificator to recognise if any interesting files have changed when it unpauses.
        // * The Reconfiguriser watches the manifest *only*. When it detects a change, it reconfigures the `watchexec`
        //   instances that underlie the Uppificator and Buildifier. There is no need to trigger a reload as
        //   both of these will already be triggered by the manifest change.
        //   * Reconfiguration is supported by the ReconfigurableWatcher, which holds the watchexec instance,
        //     and the RuntimeConfigFactory, which holds the information needed to re-read the manifest
        //     and create a new configuration for the watchexec instances.
        // * In skip_build configurations, the Buildifier is not present.
        // * In clear configurations, both the Buildifier and the Uppificator clear the screen on a change.
        //   * There is a slight twist here that the Uppificator does _not_ clear the screen if the Buildifier
        //     has just done so.  Subsequent asset changes _do_ clear the screen.

        let spin_bin = std::env::current_exe()?;
        let (manifest_file, distance) =
            spin_common::paths::find_manifest_file_path(self.app_source.as_ref())?;
        notify_if_nondefault_rel(&manifest_file, distance);

        let manifest_file = manifest_file.absolutize()?.to_path_buf(); // or watchexec misses files in subdirectories
        let manifest_dir = parent_dir(&manifest_file)?;

        // Set up the event processors (but don't start them yet).
        // We create the build-related objects even if skip_build is on - the cost is insignificant,
        // and it saves having to check options all over the place.

        let (artifact_tx, artifact_rx) = tokio::sync::watch::channel(Uuid::new_v4());
        let (pause_tx, pause_rx) = tokio::sync::mpsc::channel(1);
        let (source_code_tx, source_code_rx) = tokio::sync::watch::channel(Uuid::new_v4());
        let (manifest_tx, manifest_rx) = tokio::sync::watch::channel(Uuid::new_v4());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(Uuid::new_v4());

        let mut buildifier = Buildifier {
            spin_bin: spin_bin.clone(),
            manifest: manifest_file.clone(),
            clear_screen: self.clear,
            has_ever_built: false,
            watched_changes: source_code_rx,
            uppificator_pauser: pause_tx.clone(),
        };

        let mut uppificator = Uppificator {
            spin_bin: spin_bin.clone(),
            manifest: manifest_file.clone(),
            up_args: self.up_args.clone(),
            clear_screen: self.clear,
            watched_changes: artifact_rx,
            pause_feed: pause_rx,
            stopper: stop_rx,
        };

        // Start `watchexec` tasks to monitor artifact and build files.

        let artifact_tx = Arc::new(artifact_tx);
        let source_code_tx = Arc::new(source_code_tx);
        let manifest_tx = Arc::new(manifest_tx);

        // No need to restart on asset changes if direct-mounting

        let contains_direct_mounts = self.up_args.contains(&"--direct-mounts".to_owned());

        let artifact_filterer = Box::new(ArtifactFilterFactory {
            skip_build: self.skip_build,
            skip_assets: contains_direct_mounts,
        });
        let (artifact_watcher, artifact_watcher_handle) = self
            .spawn_watchexec(
                &manifest_file,
                &manifest_dir,
                artifact_filterer,
                artifact_tx,
                "reload",
            )
            .await
            .context("Error creating artifact watcher")?;

        let (build_watcher, build_files_watcher_handle) = if self.skip_build {
            ReconfigurableWatcher::dummy()
        } else {
            let build_filterer = Box::new(BuildFilterFactory);
            self.spawn_watchexec(
                &manifest_file,
                &manifest_dir,
                build_filterer,
                source_code_tx,
                "build",
            )
            .await
            .context("Error creating build files watcher")?
        };

        let mut reconfiguriser = reconfiguriser::Reconfiguriser {
            manifest_changes: manifest_rx,
            artifact_watcher,
            build_watcher,
        };

        let manifest_filterer = Box::new(ManifestFilterFactory);
        let (_, manifest_watcher_handle) = self
            .spawn_watchexec(
                &manifest_file,
                &manifest_dir,
                manifest_filterer,
                manifest_tx,
                "reconfigure",
            )
            .await
            .context("Error creating manifest watcher")?;

        // Start the the uppificator, buildifier and reconfiguriser to process notifications from the
        // `watchexec` tasks.

        let uppificator_handle = tokio::task::spawn(async move { uppificator.run().await });
        let buildifier_handle = if self.skip_build {
            // There's no build to complete and unpause the uppificator, so synthesise an unpause.
            let _ = buildifier.uppificator_pauser.send(Pause::Unpause).await;
            tokio::task::spawn(tokio::time::sleep(tokio::time::Duration::MAX))
        } else {
            tokio::task::spawn(async move { buildifier.run().await })
        };
        let reconfiguriser_handle = tokio::task::spawn(async move { reconfiguriser.run().await });

        // Wait for either the user to stop the watch, or some catastrophe to dump us out.

        // When the user stops the watch, signal to the Uppificator to stop `spin up` and
        // break out its run loop. This will cause the `uppificator_handle` future to
        // complete, which will cause the select to return.
        _ = ctrlc::set_handler(move || {
            let _ = stop_tx.send(Uuid::new_v4());
            // Make sure the loop that processes the stop signal sees it
            let _ = futures::executor::block_on(pause_tx.send(Pause::Unpause));
        });

        // As noted above, the most likely future to complete is the uppificator on a Ctrl+C.
        // But if any future completes, the watch can no longer continue. So we use a select
        // so that any completion causes us to fall through to program exit. (And program
        // exit kills the remaining futures.)
        _ = futures::future::select_all(vec![
            uppificator_handle,
            buildifier_handle,
            reconfiguriser_handle,
            artifact_watcher_handle,
            build_files_watcher_handle,
            manifest_watcher_handle,
        ])
        .await;

        Ok(())
    }

    // `watchexec` is normally used to restart a process on a change. In our case, we
    // manage process stops and starts manually via the Uppificator and Buildifier,
    // but watchexec still has valuable change detection functionality.  (Specifically,
    // it has the filtering feature, which didn't seem to come out of the box with things
    // like `notify`.) Therefore, we use `watchexec`, but do not set a command to run,
    // and always set the action outcome to DoNothing. Instead, our action handler manually
    // sends messages to let the two process managers make their own decisions.
    //
    // Note that this does not return the watchexec instance directly: it returns a
    // wrapper that allows self-reconfiguration (ReconfigurableWatcher), and a future
    // (tokio::JoinHandle) representing the running watchexec instance.
    async fn spawn_watchexec(
        &self,
        manifest_file: &Path,
        manifest_dir: &Path,
        filter_factory: Box<dyn FilterFactory>,
        notifier: Arc<tokio::sync::watch::Sender<Uuid>>,
        impact_description: &'static str,
    ) -> anyhow::Result<(ReconfigurableWatcher, tokio::task::JoinHandle<()>)> {
        let rtf = RuntimeConfigFactory {
            manifest_file: manifest_file.to_owned(),
            manifest_dir: manifest_dir.to_owned(),
            filter_factory,
            notifier,
            impact_description,
            debounce: Duration::from_millis(self.debounce),
        };
        let watcher = ReconfigurableWatcher::start(rtf).await?;
        Ok(watcher)
    }
}

// When the manifest changes, we need to re-generate the watchexec configuration by
// re-reading the manifest. This struct contains the info needed to fully create
// that configuration, and the logic to do so. It's more than just "where can we
// read the manifest" because the watchexec RuntimeConfig also defines the
// action on change - we can't update the filter in isolation unfortunately.
// (At least not as far as I know.)
//
// Although re-generation is the motive behind having a dedicated struct, this is
// actually used for _all_ watchexec runtime configuration, including the initial
// one. This is to ensure consistency and avoid duplicating the logic.
pub struct RuntimeConfigFactory {
    manifest_file: PathBuf,
    manifest_dir: PathBuf,
    filter_factory: Box<dyn FilterFactory>,
    notifier: Arc<tokio::sync::watch::Sender<Uuid>>,
    impact_description: &'static str,
    debounce: Duration,
}

impl RuntimeConfigFactory {
    async fn build_config(&self) -> anyhow::Result<watchexec::config::RuntimeConfig> {
        let manifest_str = tokio::fs::read_to_string(&self.manifest_file).await?;
        let manifest = spin_manifest::manifest_from_str(&manifest_str)?;
        let filterer = self
            .filter_factory
            .build_filter(&self.manifest_file, &self.manifest_dir, &manifest)
            .await?;

        let handler = NotifyOnFileChange::new(self.notifier.clone(), self.impact_description);

        let mut rt = watchexec::config::RuntimeConfig::default();
        rt.pathset([&self.manifest_dir]);
        rt.filterer(filterer);
        rt.action_throttle(self.debounce);
        rt.on_action(handler);
        Ok(rt)
    }
}

// This is the watchexec action handler that triggers the Uppificator
// to reload or Builidifer to rebuild by sending a notification.
// It is a struct rather than a closure because this makes it easier
// for the compiler to confirm that all the data lives long enough and
// is thread-safe for async stuff.
struct NotifyOnFileChange {
    despurifier: despurifier::Despurifier,
    notifier: Arc<tokio::sync::watch::Sender<Uuid>>,
    impact_description: &'static str,
}

impl NotifyOnFileChange {
    fn new(
        notifier: Arc<tokio::sync::watch::Sender<Uuid>>,
        impact_description: &'static str,
    ) -> Self {
        Self {
            despurifier: despurifier::Despurifier::new(),
            notifier,
            impact_description,
        }
    }
}

impl watchexec::handler::Handler<watchexec::action::Action> for NotifyOnFileChange {
    fn handle(
        &mut self,
        action: watchexec::action::Action,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        if self.despurifier.all_spurious(&action) {
            tracing::debug!("spin watch ignored spurious changes: {}", paths_of(&action));
        } else {
            tracing::debug!(
                "spin watch detected changes requiring {}: {}",
                self.impact_description,
                paths_of(&action)
            );
            _ = self.notifier.send(Uuid::new_v4());
        }
        action.outcome(watchexec::action::Outcome::DoNothing);
        Ok::<(), Box<dyn std::error::Error + 'static>>(())
    }
}

fn paths_of(action: &watchexec::action::Action) -> String {
    action
        .events
        .iter()
        .filter_map(path_of_event)
        .map(|p| format!("{}", p.display()))
        .unique()
        .collect_vec()
        .join(", ")
}

fn path_of_event(event: &watchexec::event::Event) -> Option<&Path> {
    event.tags.iter().filter_map(path_of_tag).next()
}

fn path_of_tag(tag: &watchexec::event::Tag) -> Option<&Path> {
    match tag {
        watchexec::event::Tag::Path { path, .. } => Some(path),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
mod despurifier {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::SystemTime;
    use watchexec::event::{filekind::FileEventKind, Tag};

    pub struct Despurifier {
        process_start_time: SystemTime,
        files_modified_at: Mutex<HashMap<String, SystemTime>>,
    }

    impl Despurifier {
        pub fn new() -> Self {
            Self {
                process_start_time: SystemTime::now(),
                files_modified_at: Mutex::new(HashMap::new()),
            }
        }

        pub fn all_spurious(&mut self, action: &watchexec::action::Action) -> bool {
            action.events.iter().all(|e| self.all_spurious_evt(e))
        }

        // This is necessary to check due to a bug on macOS emitting modify events on copies
        // https://github.com/rust-lang/rust/issues/107130
        fn all_spurious_evt(&mut self, event: &watchexec::event::Event) -> bool {
            // Deletions are never spurious
            if event
                .tags
                .iter()
                .any(|tag| matches!(tag, Tag::FileEventKind(FileEventKind::Remove(_))))
            {
                return false;
            }

            for (path, _) in event.paths() {
                let Ok(metadata) = std::fs::metadata(path) else {
                    continue;
                };
                let Ok(path_time) = metadata.modified() else {
                    continue;
                };
                let mut modified_map = self.files_modified_at.lock().unwrap();
                let path_key = match path.to_str() {
                    Some(s) => s.to_owned(),
                    None => {
                        tracing::warn!("can't check non-unicode path: {path:?}");
                        continue;
                    }
                };
                let base_time = modified_map
                    .get(&path_key)
                    .unwrap_or(&self.process_start_time);
                if &path_time > base_time {
                    modified_map.insert(path_key, path_time);
                    return false;
                }
            }

            true
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod despurifier {
    pub struct Despurifier;

    impl Despurifier {
        pub fn new() -> Self {
            Self
        }

        pub fn all_spurious(&mut self, _action: &watchexec::action::Action) -> bool {
            false
        }
    }
}

// On manifest change, the Reconfiguriser updates the watchexec configuration
// for the Uppificator and Buildifier. This means that we need to hold onto both
// the watchexec instance (so we can call reconfigure on it) *and* the logic for
// building a configuration from a manifest (so we know what to reconfigure it *to*).
// ReconfigurableWatcher wraps those up, and returns the future (JoinHandle) that
// we also need to wait on the watchexec tasks.
pub(crate) enum ReconfigurableWatcher {
    Actual((Arc<Watchexec>, RuntimeConfigFactory)),
    Dummy,
}

impl ReconfigurableWatcher {
    async fn start(
        rtf: RuntimeConfigFactory,
    ) -> anyhow::Result<(Self, tokio::task::JoinHandle<()>)> {
        let rt = rtf.build_config().await?;
        let watcher = Watchexec::new(watchexec::config::InitConfig::default(), rt)?;
        let watcher_clone = watcher.clone();
        let join_handle = tokio::task::spawn(async move {
            _ = watcher_clone.main().await;
        });

        Ok((Self::Actual((watcher, rtf)), join_handle))
    }

    fn dummy() -> (Self, tokio::task::JoinHandle<()>) {
        let join_handle = tokio::task::spawn(tokio::time::sleep(tokio::time::Duration::MAX));
        (Self::Dummy, join_handle)
    }

    pub async fn reconfigure(&self) {
        match self {
            Self::Actual((watchexec, rtf)) => {
                let rt = match rtf.build_config().await {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("Unable to re-configure watcher after manifest change. Changes in files newly added to the application may not be detected. Error: {e}");
                        return;
                    }
                };
                if let Err(e) = watchexec.reconfigure(rt) {
                    tracing::error!("Unable to re-configure watcher after manifest change. Changes in files newly added to the application may not be detected. Error: {e}");
                }
            }
            Self::Dummy => (),
        }
    }
}
