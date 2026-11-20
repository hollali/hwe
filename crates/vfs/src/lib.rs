pub mod entry;
pub mod error;
pub mod local;
pub mod path;

pub use entry::{Capabilities, DirEntry, EntryKind, Metadata, RemoveMode};
pub use error::{Result, VfsError};
pub use local::LocalFs;
pub use path::VPath;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct DirStream {
    rx: mpsc::Receiver<Result<DirEntry>>,
}

impl DirStream {
    pub(crate) fn new(rx: mpsc::Receiver<Result<DirEntry>>) -> Self {
        Self { rx }
    }

    pub async fn next(&mut self) -> Option<Result<DirEntry>> {
        self.rx.recv().await
    }
}

#[async_trait]
pub trait Vfs: Send + Sync {
    async fn read_dir(&self, path: &VPath) -> Result<DirStream>;

    async fn stat(&self, path: &VPath) -> Result<Metadata>;

    async fn open_read(&self, path: &VPath) -> Result<Box<dyn AsyncRead + Send + Unpin>>;

    async fn open_write(&self, path: &VPath) -> Result<Box<dyn AsyncWrite + Send + Unpin>>;

    async fn create_dir(&self, path: &VPath) -> Result<()>;

    async fn create_file(&self, path: &VPath) -> Result<()>;

    async fn copy(&self, from: &VPath, to: &VPath) -> Result<()>;

    async fn rename(&self, from: &VPath, to: &VPath) -> Result<()>;

    async fn remove(&self, path: &VPath, mode: RemoveMode) -> Result<()>;

    fn capabilities(&self) -> Capabilities;
}
