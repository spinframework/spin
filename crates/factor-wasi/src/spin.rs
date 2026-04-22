use std::path::PathBuf;

use spin_common::{ui::quoted_path, url::parse_file_url};
use spin_factors::anyhow::{Context, ensure};

use crate::FilesMounter;

pub struct SpinFilesMounter {
    working_dir: PathBuf,
    allow_transient_writes: bool,
}

impl SpinFilesMounter {
    pub fn new(working_dir: impl Into<PathBuf>, allow_transient_writes: bool) -> Self {
        Self {
            working_dir: working_dir.into(),
            allow_transient_writes,
        }
    }
}

impl FilesMounter for SpinFilesMounter {
    fn mount_files(
        &self,
        app_component: &spin_factors::AppComponent,
        mut ctx: crate::MountFilesContext,
    ) -> spin_factors::anyhow::Result<()> {
        for content_dir in app_component.files() {
            let source_uri = content_dir
                .content
                .source
                .as_deref()
                .with_context(|| format!("Missing 'source' on files mount {content_dir:?}"))?;
            let source_path = self.working_dir.join(parse_file_url(source_uri)?);
            ensure!(
                source_path.is_dir(),
                "SpinFilesMounter only supports directory mounts; {} is not a directory",
                quoted_path(&source_path),
            );
            let guest_path = &content_dir.path;
            let guest_path = guest_path
                .to_str()
                .with_context(|| format!("guest path {guest_path:?} not valid UTF-8"))?;
            ctx.preopened_dir(source_path, guest_path, self.allow_transient_writes)?;
        }
        Ok(())
    }
}
