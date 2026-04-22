use std::path::Path;

use anyhow::{Context, Result, ensure};
use sha2::Digest;
use tokio::io::AsyncWriteExt;

/// Describes the naming convention that `verified_download` is permitted
/// to assume in the directory it saves downloads to.
///
/// Consumers (direct or indirect) of `verified_download` are expected to check
/// if the file is already downloaded before calling it. This enum exists
/// to address race conditions when the same blob is downloaded several times
/// concurrently.
///
/// The significance of this is for when the destination file turns out to already
/// exist after all (that is, has been created since the caller originally checked
/// existence). In the ContentIndexed case, the name already existing guarantees that
/// the file matches the download. If a caller uses `verified_download` for a
/// *non*-content-indexed case then they must provide and handle a new variant
/// of the enum.
pub enum DestinationConvention {
    /// The download destination is content-indexed; therefore, in the event
    /// of a race, the loser of the race can be safely discarded.
    ContentIndexed,
}

/// Downloads content from `url` which will be verified to match `digest` and
/// then moved to `dest`.
pub async fn verified_download(
    url: &str,
    digest: &str,
    dest: &Path,
    convention: DestinationConvention,
) -> Result<()> {
    tracing::debug!("Downloading content from {url:?}");

    // Prepare tempfile destination
    let prefix = format!("download-{}", digest.replace(':', "-"));
    let dest_dir = dest.parent().context("invalid dest")?;
    let (temp_file, temp_path) = tempfile::NamedTempFile::with_prefix_in(prefix, dest_dir)
        .context("error creating download tempfile")?
        .into_parts();

    // Begin download
    let mut resp = reqwest::get(url).await?.error_for_status()?;

    // Hash as we write to the tempfile
    let mut hasher = sha2::Sha256::new();
    {
        let mut temp_file = tokio::fs::File::from_std(temp_file);
        while let Some(chunk) = resp.chunk().await? {
            hasher.update(&chunk);
            temp_file.write_all(&chunk).await?;
        }
        temp_file.flush().await?;
    }

    // Check the digest
    let actual_digest = format!("sha256:{:x}", hasher.finalize());
    ensure!(
        actual_digest == digest,
        "invalid content digest; expected {digest}, downloaded {actual_digest}"
    );

    // Move to final destination
    let persist_result = temp_path.persist_noclobber(dest);

    persist_result.or_else(|e| {
        let file_already_exists = e.error.kind() == std::io::ErrorKind::AlreadyExists;
        if file_already_exists && matches!(convention, DestinationConvention::ContentIndexed) {
            Ok(())
        } else {
            Err(e).with_context(|| {
                format!("Failed to save download from {url} to {}", dest.display())
            })
        }
    })
}
