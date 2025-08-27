use command_group::AsyncCommandGroup;
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) struct Uppificator {
    pub spin_bin: PathBuf,
    pub up_args: Vec<String>,
    pub manifest: PathBuf,
    pub profile: Option<String>,
    pub clear_screen: bool,
    pub watched_changes: tokio::sync::watch::Receiver<Uuid>,
    pub pause_feed: tokio::sync::mpsc::Receiver<Pause>,
    pub stopper: tokio::sync::watch::Receiver<Uuid>,
}

#[derive(Debug)]
pub(crate) enum Pause {
    Pause,
    Unpause,
}

enum UppificatorAction {
    Restart,
    Resume,
    Stop,
    Wait,
}

impl Uppificator {
    pub(crate) async fn run(&mut self) {
        // Wait for first build to complete. (If skip_build is set, spin watch
        // sends a synthetic unpause.)
        loop {
            let p = self.pause_feed.recv().await;
            if matches!(p, Some(Pause::Unpause)) {
                break;
            }
        }

        'run: loop {
            let mut cmd = tokio::process::Command::new(&self.spin_bin);
            cmd.arg("up")
                .arg("-f")
                .arg(&self.manifest)
                .args(&self.up_args);
            if let Some(profile) = &self.profile {
                cmd.arg("--profile").arg(profile);
            }
            let mut child = match cmd.group_spawn() {
                Ok(ch) => ch,
                Err(e) => {
                    tracing::error!("Can't launch `spin up`: {e:#}");
                    break 'run;
                }
            };

            let mut resuming_after_build = false;

            loop {
                match self.next_event(&mut child).await {
                    UppificatorAction::Restart => break,
                    UppificatorAction::Resume => {
                        resuming_after_build = true;
                        continue;
                    }
                    UppificatorAction::Stop => break 'run,
                    UppificatorAction::Wait => continue,
                }
            }

            if self.clear_screen && !resuming_after_build {
                _ = clearscreen::clear();
            }
        }
    }

    async fn next_event(
        &mut self,
        child: &mut command_group::AsyncGroupChild,
    ) -> UppificatorAction {
        tokio::select! {
            _ = child.wait() => {
                UppificatorAction::Wait
            },
            _ = self.watched_changes.changed() => {
                stop(child).await;
                UppificatorAction::Restart
            },
            p = self.pause_feed.recv() => {
                if matches!(p, Some(Pause::Pause)) {
                    loop {
                        match self.pause_feed.recv().await {
                            Some(Pause::Unpause) => return UppificatorAction::Resume,
                            _ => continue,
                        }
                    }
                } else {
                    UppificatorAction::Resume
                }
            }
            _ = self.stopper.changed() => {
                stop(child).await;
                UppificatorAction::Stop
            }
        }
    }
}

#[cfg(unix)]
async fn stop(child: &mut command_group::AsyncGroupChild) {
    if let Some(child_id) = child.id() {
        let pid = nix::unistd::Pid::from_raw(child_id as i32);
        if let Err(e) = nix::sys::signal::kill(pid, Some(nix::sys::signal::Signal::SIGINT)) {
            tracing::warn!("Could not send interrupt signal to child process: {e:#}");
            _ = child.kill();
        }
    } else if let Err(e) = child.kill() {
        tracing::warn!("Could not terminate child process: {e:#}");
    }
    _ = child.wait().await;
}

#[cfg(not(unix))]
async fn stop(child: &mut command_group::AsyncGroupChild) {
    if let Err(e) = child.kill() {
        tracing::warn!("Could not terminate child process: {e:#}");
    }
    _ = child.wait().await;
}
