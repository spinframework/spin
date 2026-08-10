//! Utilities related to distributing Spin apps via OCI registries

use anyhow::{Context, Result};
use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder;
use spin_common::ui::quoted_path;
use std::path::{Path, PathBuf};
use tokio_tar::Archive;

/// Create a compressed archive of source, returning its path in working_dir
pub async fn archive(source: &Path, working_dir: &Path) -> Result<PathBuf> {
    let source = source.to_owned();
    let working_dir = working_dir.to_owned();

    // Create tar archive file
    let tar_gz_path = working_dir
        .join(source.file_name().unwrap())
        .with_extension("tar.gz");
    let tar_gz = tokio::fs::File::create(tar_gz_path.as_path())
        .await
        .context(format!(
            "Unable to create tar archive for source {}",
            quoted_path(&source)
        ))?;

    // Create encoder
    // TODO: use zstd? May be more performant
    let tar_gz_enc = GzipEncoder::new(tar_gz);

    // Build tar archive
    let mut tar_builder = tokio_tar::Builder::new(tar_gz_enc);
    tar_builder
        .append_dir_all(".", &source)
        .await
        .context(format!(
            "Unable to create tar archive for source {}",
            quoted_path(&source)
        ))?;

    // Finish writing the archive
    tar_builder.finish().await?;

    // Shut down the encoder
    use tokio::io::AsyncWriteExt;
    tar_builder
        .into_inner()
        .await?
        .into_inner()
        .shutdown()
        .await?;

    Ok(tar_gz_path)
}

/// Unpack a compressed archive existing at source into dest
pub async fn unarchive(source: &Path, dest: &Path) -> Result<()> {
    let source = source.to_owned();
    let dest = dest.to_owned();

    let decoder = GzipDecoder::new(tokio::io::BufReader::new(
        tokio::fs::File::open(&source).await?,
    ));
    let mut archive = Archive::new(decoder);
    if let Err(e) = archive.unpack(&dest).await {
        return Err(e.into());
    };
    Ok(())
}
