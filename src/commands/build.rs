use std::{ffi::OsString, path::PathBuf};

use anyhow::Result;
use clap::Parser;

use crate::{
    directory_rels::notify_if_nondefault_rel,
    opts::{APP_MANIFEST_FILE_OPT, BUILD_UP_OPT},
};

use super::up::UpCommand;

/// Run the build command for each component.
#[derive(Parser, Debug)]
#[clap(about = "Build the Spin application", allow_hyphen_values = true)]
pub struct BuildCommand {
    /// The application to build. This may be a manifest (spin.toml) file, or a
    /// directory containing a spin.toml file.
    /// If omitted, it defaults to "spin.toml".
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
    )]
    pub app_source: Option<PathBuf>,

    /// Component ID to build. This can be specified multiple times. The default is all components.
    #[clap(short = 'c', long, multiple = true)]
    pub component_id: Vec<String>,

    /// By default, if the application manifest specifies one or more deployment targets, Spin
    /// checks that all components are compatible with those deployment targets. Specify
    /// this option to bypass those target checks.
    #[clap(
        long = "skip-target-checks",
        alias = "skip-target-check",
        takes_value = false
    )]
    skip_target_checks: bool,

    /// Run the application after building.
    #[clap(name = BUILD_UP_OPT, short = 'u', long = "up")]
    pub up: bool,

    #[clap(requires = BUILD_UP_OPT)]
    pub up_args: Vec<OsString>,
}

impl BuildCommand {
    pub async fn run(self) -> Result<()> {
        let (manifest_file, distance) =
            spin_common::paths::find_manifest_file_path(self.app_source.as_ref())?;
        notify_if_nondefault_rel(&manifest_file, distance);

        spin_build::build(
            &manifest_file,
            &self.component_id,
            self.target_checking(),
            None,
        )
        .await?;

        if self.up {
            let mut cmd = UpCommand::parse_from(
                std::iter::once(OsString::from(format!(
                    "{} up",
                    std::env::args().next().unwrap()
                )))
                .chain(self.up_args),
            );
            cmd.file_source = Some(manifest_file);
            cmd.run().await
        } else {
            Ok(())
        }
    }

    fn target_checking(&self) -> spin_build::TargetChecking {
        if self.skip_target_checks {
            spin_build::TargetChecking::Skip
        } else {
            spin_build::TargetChecking::Check
        }
    }
}
