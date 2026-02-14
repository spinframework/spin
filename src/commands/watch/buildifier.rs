use command_group::tokio::AsyncCommandGroup;
use std::path::PathBuf;
use uuid::Uuid;

use super::uppificator::Pause;

pub(crate) struct Buildifier {
    pub spin_bin: PathBuf,
    pub manifest: PathBuf,
    pub profile: Option<String>,
    pub clear_screen: bool,
    pub has_ever_built: bool,
    pub watched_changes: tokio::sync::watch::Receiver<Uuid>, // TODO: refine which component(s) a change affects
    pub uppificator_pauser: tokio::sync::mpsc::Sender<Pause>,
}

impl Buildifier {
    #[allow(clippy::collapsible_if)]
    pub(crate) async fn run(&mut self) {
        // Other components may close channels as part of shutdown, so if any channels
        // fail, just exit the loop and fall out normally.

        loop {
            if self.clear_screen {
                _ = clearscreen::clear();
            }

            if self.uppificator_pauser.send(Pause::Pause).await.is_err() {
                break;
            }

            let build_result = self.build_once().await;
            if !self.has_ever_built {
                self.has_ever_built = matches!(build_result, Ok(true));
            }

            if self.has_ever_built {
                if self.uppificator_pauser.send(Pause::Unpause).await.is_err() {
                    break;
                }
            }

            if self.watched_changes.changed().await.is_err() {
                break;
            }
        }
    }

    pub(crate) async fn build_once(&mut self) -> std::io::Result<bool> {
        loop {
            let mut cmd = tokio::process::Command::new(&self.spin_bin);
            cmd.arg("build").arg("-f").arg(&self.manifest);
            if let Some(profile) = &self.profile {
                cmd.arg("--profile").arg(profile);
            }
            let mut child = cmd.group_spawn()?;

            tokio::select! {
                exit_status = child.wait() => {
                    // It reports its own errors so we only care about success or failure (and then only for
                    // the initial build).
                    return Ok(exit_status?.success());
                }
                _ = self.watched_changes.changed() => {
                    tracing::debug!("Cancelling build as there are new changes to process");
                    child.kill().await?;
                    if self.clear_screen {
                        _ = clearscreen::clear();
                    }
                    continue;
                }

            }
        }
    }
}
