//! Utilities related to distributing Spin apps via OCI registries

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use spin_common::ui::quoted_path;
use std::path::{Path, PathBuf};
use tar::Archive;

/// Create a compressed archive of source, returning its path in working_dir
pub async fn archive(source: &Path, working_dir: &Path) -> Result<PathBuf> {
    let source = source.to_owned();
    let working_dir = working_dir.to_owned();

    tokio::task::spawn_blocking(move || {
        // Create tar archive file
        let tar_gz_path = working_dir
            .join(source.file_name().unwrap())
            .with_extension("tar.gz");
        let tar_gz = std::fs::File::create(tar_gz_path.as_path()).context(format!(
            "Unable to create tar archive for source {}",
            quoted_path(&source)
        ))?;

        // Create encoder
        // TODO: use zstd? May be more performant
        let tar_gz_enc = GzEncoder::new(tar_gz, flate2::Compression::default());

        // Build tar archive
        let mut tar_builder = tar::Builder::new(tar_gz_enc);
        tar_builder.append_dir_all(".", &source).context(format!(
            "Unable to create tar archive for source {}",
            quoted_path(&source)
        ))?;

        // Finish writing the archive and shut down the encoder.
        let inner_enc = tar_builder.into_inner()?;
        inner_enc.finish()?;

        Ok(tar_gz_path)
    })
    .await?
}

/// Unpack a compressed archive existing at source into dest
pub async fn unarchive(source: &Path, dest: &Path) -> Result<()> {
    let source = source.to_owned();
    let dest = dest.to_owned();

    tokio::task::spawn_blocking(move || {
        let decoder = GzDecoder::new(std::fs::File::open(&source)?);
        let mut archive = Archive::new(decoder);
        if let Err(e) = archive.unpack(&dest) {
            return Err(e.into());
        };
        Ok(())
    })
    .await?
}
