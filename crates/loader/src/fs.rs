use anyhow::Result;
use spin_common::fs::temp_file_beside;
use std::path::Path;

#[cfg(feature = "async-io")]
mod io {
    use super::*;

    /// Writes `bytes` to a temporary file that is then renamed over `path`, so a
    /// concurrent reader never sees a partially written file.
    pub async fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
        let tmp = temp_file_beside(path)?;
        tokio::fs::write(tmp.path(), bytes).await?;
        tmp.persist(path)?;
        Ok(())
    }

    pub async fn create_dir_all(path: &Path) -> Result<()> {
        tokio::fs::create_dir_all(path).await?;
        Ok(())
    }

    pub async fn copy(from: &Path, to: &Path) -> Result<u64> {
        tokio::fs::copy(from, to).await.map_err(Into::into)
    }

    pub async fn metadata(path: &Path) -> Result<std::fs::Metadata> {
        tokio::fs::metadata(path).await.map_err(Into::into)
    }
}

#[cfg(not(feature = "async-io"))]
mod io {
    use super::*;

    /// Writes `bytes` to a temporary file that is then renamed over `path`, so a
    /// concurrent reader never sees a partially written file.
    pub async fn write_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
        let tmp = temp_file_beside(path)?;
        std::fs::write(tmp.path(), bytes)?;
        tmp.persist(path)?;
        Ok(())
    }

    pub async fn create_dir_all(path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)?;
        Ok(())
    }

    pub async fn copy(from: &Path, to: &Path) -> Result<u64> {
        Ok(std::fs::copy(from, to)?)
    }

    pub async fn metadata(path: &Path) -> Result<std::fs::Metadata> {
        Ok(std::fs::metadata(path)?)
    }
}

pub use io::*;
