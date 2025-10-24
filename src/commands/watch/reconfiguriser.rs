use uuid::Uuid;

pub(crate) struct Reconfiguriser {
    pub manifest_changes: tokio::sync::watch::Receiver<(Uuid, String)>,
    pub artifact_watcher: super::ReconfigurableWatcher,
    pub build_watcher: super::ReconfigurableWatcher,
}

impl Reconfiguriser {
    pub(crate) async fn run(&mut self) {
        loop {
            if self.manifest_changes.changed().await.is_err() {
                break;
            }

            self.artifact_watcher.reconfigure().await;
            self.build_watcher.reconfigure().await;
        }
    }
}
