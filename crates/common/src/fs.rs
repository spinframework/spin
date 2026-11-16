//! Filesystem helpers

use anyhow::Context;
use std::path::Path;

use crate::ui::quoted_path;

/// Creates a temporary file in the directory of `path`, to be persisted over it.
/// On Unix the file is created with mode 0666 less the umask, as `std::fs::write`
/// would create it, rather than tempfile's default of 0600.
pub fn temp_file_beside(path: &Path) -> anyhow::Result<tempfile::NamedTempFile> {
    let dir = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    builder
        .tempfile_in(dir)
        .with_context(|| format!("cannot create temporary file in {}", dir.display()))
}

/// Writes `bytes` to `path` through a temporary file in the same directory
/// that is then renamed over `path`, so a concurrent reader sees either the
/// previous content or the new content, never a partial file.
pub async fn write_file_atomically_stream(
    path: &Path,
    bytes: impl futures::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Unpin,
) -> anyhow::Result<()> {
    let tmp = temp_file_beside(path)?;
    let mut bytes = tokio_util::io::StreamReader::new(bytes);
    let mut f = tokio::fs::File::create(tmp.path()).await?;
    tokio::io::copy(&mut bytes, &mut f).await?;
    tmp.persist(path)
        .with_context(|| format!("cannot write {}", quoted_path(path)))?;
    Ok(())
}
