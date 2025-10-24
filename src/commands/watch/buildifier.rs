use command_group::AsyncCommandGroup;
use std::{collections::HashMap, path::PathBuf};
use uuid::Uuid;

use super::uppificator::Pause;
#[derive(Debug)]
pub(crate) struct Buildifier {
    pub spin_bin: PathBuf,
    pub manifest: PathBuf,
    pub clear_screen: bool,
    pub has_ever_built: bool,
    pub watched_changes: tokio::sync::watch::Receiver<(Uuid, String)>, // TODO: refine which component(s) a change affects
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

            let (_, ref changed_path) = self.watched_changes.borrow_and_update().clone();
            tracing::debug!("Detected changes in: {:?}", changed_path);

            let build_component_result = self.build_component(changed_path).await;

            if !self.has_ever_built {
                self.has_ever_built = matches!(build_component_result, Ok(true));
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

    pub(crate) async fn build_component(&mut self, component_path: &str) -> std::io::Result<bool> {
        let manifest = spin_manifest::manifest_from_file(&self.manifest).unwrap();

        let id_to_workdir: HashMap<_, _> = manifest
            .components
            .iter()
            .filter_map(|(id, component)| {
                component.build.as_ref().map(|build_config| {
                    (
                        id.as_ref(),
                        build_config.workdir.clone().unwrap_or("".to_owned()),
                    )
                })
            })
            .collect();

        let component_paths: Vec<&str> = component_path.split(", ").collect();
        let mut component_ids = Vec::new();
        let source_dir = id_to_workdir
            .iter()
            .find(|(_, value)| value.is_empty())
            .map(|(key, _)| key);

        for changed_path in &component_paths {
            for (inner_id, workdir) in id_to_workdir.iter() {
                if !workdir.is_empty() && changed_path.contains(workdir) {
                    component_ids.push(inner_id);
                    break;
                }
            }
        }

        if component_ids.len() != component_paths.len() {
            component_ids.push(source_dir.unwrap());
        }

        loop {
            let mut cmd = tokio::process::Command::new(&self.spin_bin);

            if component_paths.contains(&"THIS_IS_ THE-FIRST BUILD")
                || component_paths.contains(&self.manifest.to_str().unwrap())
            {
                cmd.arg("build").arg("-f").arg(&self.manifest);
            } else {
                cmd.arg("build")
                    .arg("-c")
                    .args(&component_ids)
                    .arg("-f")
                    .arg(&self.manifest);
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
                        child.kill()?;
                        if self.clear_screen {
                            _ = clearscreen::clear();
                        }
                        continue;
                    }
            }
        }
    }
}
