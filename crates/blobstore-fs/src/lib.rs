use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use spin_core::async_trait;
use spin_factor_blobstore::runtime_config::spin::MakeBlobStore;

/// A blob store that uses a persistent file system volume
/// as a back end.
#[derive(Default)]
pub struct FileSystemBlobStore {
    _priv: (),
}

impl FileSystemBlobStore {
    /// Creates a new `FileSystemBlobStore`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MakeBlobStore for FileSystemBlobStore {
    const RUNTIME_CONFIG_TYPE: &'static str = "file_system";

    type RuntimeConfig = FileSystemBlobStoreRuntimeConfig;

    type ContainerManager = BlobStoreFileSystem;

    fn make_store(
        &self,
        runtime_config: Self::RuntimeConfig,
    ) -> anyhow::Result<Self::ContainerManager> {
        Ok(BlobStoreFileSystem::new(runtime_config.path))
    }
}

pub struct BlobStoreFileSystem {
    path: PathBuf,
}

impl BlobStoreFileSystem {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

/// The serialized runtime configuration for the in memory blob store.
#[derive(Deserialize, Serialize)]
pub struct FileSystemBlobStoreRuntimeConfig {
    path: PathBuf,
}

#[async_trait]
impl spin_factor_blobstore::ContainerManager for BlobStoreFileSystem {
    async fn get(&self, name: &str) -> Result<Arc<dyn spin_factor_blobstore::Container>, String> {
        let container = FileSystemContainer::new(name, &self.path);
        Ok(Arc::new(container))
    }

    fn is_defined(&self, _container_name: &str) -> bool {
        true
    }
}

struct FileSystemContainer {
    name: String,
    path: PathBuf,
}

impl FileSystemContainer {
    fn new(name: &str, path: &Path) -> Self {
        Self {
            name: name.to_string(),
            path: path.to_owned(),
        }
    }

    fn object_path(&self, name: &str) -> anyhow::Result<PathBuf> {
        validate_no_escape(name)?;
        Ok(self.path.join(name))
    }
}

fn validate_no_escape(name: &str) -> anyhow::Result<()> {
    // TODO: this is hopelessly naive but will do for testing
    if name.contains("..") {
        anyhow::bail!("path tries to escape from base directory");
    }
    Ok(())
}

#[async_trait]
impl spin_factor_blobstore::Container for FileSystemContainer {
    async fn exists(&self) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn name(&self) -> String {
        self.name.clone()
    }
    async fn info(&self) -> anyhow::Result<spin_factor_blobstore::ContainerMetadata> {
        let meta = self.path.metadata()?;
        let created_at = created_at_nanos(&meta)?;

        Ok(spin_factor_blobstore::ContainerMetadata {
            name: self.name.to_owned(),
            created_at,
        })
    }
    async fn clear(&self) -> anyhow::Result<()> {
        let entries = std::fs::read_dir(&self.path)?.collect::<Vec<_>>();

        for entry in entries {
            let entry = entry?;
            if entry.metadata()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }

        Ok(())
    }
    async fn delete_object(&self, name: &str) -> anyhow::Result<()> {
        tokio::fs::remove_file(self.object_path(name)?).await?;
        Ok(())
    }
    async fn delete_objects(&self, names: &[String]) -> anyhow::Result<()> {
        let futs = names.iter().map(|name| self.delete_object(name));
        let results = futures::future::join_all(futs).await;

        if let Some(err_result) = results.into_iter().find(|r| r.is_err()) {
            err_result
        } else {
            Ok(())
        }
    }
    async fn has_object(&self, name: &str) -> anyhow::Result<bool> {
        Ok(self.object_path(name)?.exists())
    }
    async fn object_info(
        &self,
        name: &str,
    ) -> anyhow::Result<spin_factor_blobstore::ObjectMetadata> {
        let meta = tokio::fs::metadata(self.object_path(name)?).await?;
        let created_at = created_at_nanos(&meta)?;
        Ok(spin_factor_blobstore::ObjectMetadata {
            name: name.to_string(),
            container: self.name.to_string(),
            created_at,
            size: meta.len(),
        })
    }
    async fn get_data(
        &self,
        name: &str,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Box<dyn spin_factor_blobstore::IncomingData>> {
        let path = self.object_path(name)?;
        let file = tokio::fs::File::open(&path).await?;

        Ok(Box::new(BlobContent {
            file: Some(file),
            start,
            end,
        }))
    }

    async fn write_data(
        &self,
        name: &str,
        data: tokio::io::ReadHalf<tokio::io::SimplexStream>,
        finished_tx: tokio::sync::mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        let path = self.object_path(name)?;
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let file = tokio::fs::File::create(&path).await?;

        tokio::spawn(async move {
            let write_result = Self::write_data_core(data, file).await;
            finished_tx
                .send(write_result)
                .await
                .expect("shoulda sent finished_tx");
        });

        Ok(())
    }

    async fn list_objects(&self) -> anyhow::Result<Box<dyn spin_factor_blobstore::ObjectNames>> {
        if !self.path.is_dir() {
            anyhow::bail!(
                "Backing store for {} does not exist or is not a directory",
                self.name
            );
        }
        Ok(Box::new(BlobNames::new(&self.path)))
    }
}

impl FileSystemContainer {
    async fn write_data_core(
        data: tokio::io::ReadHalf<tokio::io::SimplexStream>,
        file: tokio::fs::File,
    ) -> anyhow::Result<()> {
        use futures::SinkExt;
        use tokio_util::codec::{BytesCodec, FramedWrite};

        // Ceremonies to turn `file` and `data` into Sink and Stream
        let mut file_sink = FramedWrite::new(file, BytesCodec::new());
        let mut data_stm = tokio_util::io::ReaderStream::new(data);

        file_sink.send_all(&mut data_stm).await?;

        Ok(())
    }
}

struct BlobContent {
    file: Option<tokio::fs::File>,
    start: u64,
    end: u64,
}

#[async_trait]
impl spin_factor_blobstore::IncomingData for BlobContent {
    async fn consume_sync(&mut self) -> anyhow::Result<Vec<u8>> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let mut file = self.file.take().context("already consumed")?;

        let mut buf = Vec::with_capacity(1000);

        file.seek(std::io::SeekFrom::Start(self.start)).await?;
        file.take(self.end - self.start)
            .read_to_end(&mut buf)
            .await?;

        Ok(buf)
    }

    fn consume_async(&mut self) -> wasmtime_wasi::p2::pipe::AsyncReadStream {
        use futures::StreamExt;
        use futures::TryStreamExt;
        use tokio_util::compat::FuturesAsyncReadCompatExt;

        let file = self.file.take().unwrap();
        let stm = tokio_util::io::ReaderStream::new(file)
            .skip(self.start.try_into().unwrap())
            .take((self.end - self.start).try_into().unwrap());

        let ar = stm.into_async_read().compat();
        wasmtime_wasi::p2::pipe::AsyncReadStream::new(ar)
    }

    async fn size(&mut self) -> anyhow::Result<u64> {
        let file = self.file.as_ref().context("already consumed")?;
        let meta = file.metadata().await?;
        Ok(meta.len())
    }
}

struct BlobNames {
    // This isn't async like tokio ReadDir, but it saves us having
    // to manage state ourselves as we traverse into subdirectories.
    walk_dir: Box<dyn Iterator<Item = Result<PathBuf, walkdir::Error>> + Send + Sync>,

    base_path: PathBuf,
}

impl BlobNames {
    fn new(path: &Path) -> Self {
        let walk_dir = walkdir::WalkDir::new(path)
            .into_iter()
            .filter_map(as_file_path);
        Self {
            walk_dir: Box::new(walk_dir),
            base_path: path.to_owned(),
        }
    }

    fn object_name(&self, path: &Path) -> anyhow::Result<String> {
        Ok(path
            .strip_prefix(&self.base_path)
            .map(|p| format!("{}", p.display()))?)
    }
}

fn as_file_path(
    entry: Result<walkdir::DirEntry, walkdir::Error>,
) -> Option<Result<PathBuf, walkdir::Error>> {
    match entry {
        Err(err) => Some(Err(err)),
        Ok(entry) => {
            if entry.file_type().is_file() {
                Some(Ok(entry.into_path()))
            } else {
                None
            }
        }
    }
}

#[async_trait]
impl spin_factor_blobstore::ObjectNames for BlobNames {
    async fn read(&mut self, len: u64) -> anyhow::Result<(Vec<String>, bool)> {
        let mut names = Vec::with_capacity(len.try_into().unwrap_or_default());
        let mut at_end = false;

        for _ in 0..len {
            match self.walk_dir.next() {
                None => {
                    at_end = true;
                    break;
                }
                Some(Err(e)) => {
                    anyhow::bail!(e);
                }
                Some(Ok(path)) => {
                    names.push(self.object_name(&path)?);
                }
            }
        }

        // We could report "at end" when we actually just returned the last file.
        // It's not worth messing around with peeking ahead because the cost to the
        // guest of making a call that returns nothing is (hopefully) small.
        Ok((names, at_end))
    }

    async fn skip(&mut self, num: u64) -> anyhow::Result<(u64, bool)> {
        // TODO: we could save semi-duplicate code by delegating to `read`?
        // The cost would be a bunch of allocation but that seems minor when
        // you're dealing with the filesystem.

        let mut count = 0;
        let mut at_end = false;

        for _ in 0..num {
            match self.walk_dir.next() {
                None => {
                    at_end = true;
                    break;
                }
                Some(Err(e)) => {
                    anyhow::bail!(e);
                }
                Some(Ok(_)) => {
                    count += 1;
                }
            }
        }

        Ok((count, at_end))
    }
}

fn created_at_nanos(meta: &std::fs::Metadata) -> anyhow::Result<u64> {
    let time_nanos = meta
        .created()?
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_nanos()
        .try_into()
        .unwrap_or_default();
    Ok(time_nanos)
}
