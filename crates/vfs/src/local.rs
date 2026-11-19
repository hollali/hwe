use std::fs;
use std::path::Path;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Capabilities, DirEntry, DirStream, EntryKind, Metadata, RemoveMode, Result, VPath, Vfs,
    VfsError,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFs;

impl LocalFs {
    pub fn new() -> Self {
        Self
    }
}

fn build_entry(parent: &Path, raw: &fs::DirEntry) -> Result<DirEntry> {
    let name_os = raw.file_name();
    let name = name_os.to_string_lossy().into_owned();
    let path = VPath::new(parent.join(&name_os));
    let file_type = raw
        .file_type()
        .map_err(|error| VfsError::from_io(error, path.as_path()))?;
    let is_symlink = file_type.is_symlink();
    let metadata = raw.metadata().ok();
    let kind = match &metadata {
        Some(metadata) if metadata.is_dir() => EntryKind::Dir,
        Some(metadata) if metadata.is_file() => EntryKind::File,
        _ => EntryKind::Other,
    };
    let size = match &metadata {
        Some(metadata) if !metadata.is_dir() => Some(metadata.len()),
        _ => None,
    };
    #[cfg(unix)]
    let is_executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata
            .as_ref()
            .is_some_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    #[cfg(not(unix))]
    let is_executable = false;

    let modified = metadata.and_then(|metadata| metadata.modified().ok());
    let is_hidden = name.starts_with('.');

    Ok(DirEntry {
        name,
        path,
        kind,
        size,
        modified,
        is_hidden,
        is_symlink,
        is_executable,
    })
}

fn read_dir_blocking(
    base: std::path::PathBuf,
    tx: mpsc::Sender<Result<DirEntry>>,
    ready: oneshot::Sender<Result<()>>,
) {
    let reader = match fs::read_dir(&base) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = ready.send(Err(VfsError::from_io(error, &base)));
            return;
        }
    };

    if ready.send(Ok(())).is_err() {
        return;
    }

    for item in reader {
        if tx.is_closed() {
            break;
        }
        let result = item
            .map_err(|error| VfsError::from_io(error, &base))
            .and_then(|raw| build_entry(&base, &raw));
        if tx.blocking_send(result).is_err() {
            break;
        }
    }
}

#[async_trait]
impl Vfs for LocalFs {
    async fn read_dir(&self, path: &VPath) -> Result<DirStream> {
        let base = path.as_path().to_path_buf();
        let (tx, rx) = mpsc::channel(256);
        let (ready_tx, ready_rx) = oneshot::channel();

        tokio::task::spawn_blocking(move || read_dir_blocking(base, tx, ready_tx));

        match ready_rx.await {
            Ok(Ok(())) => Ok(DirStream::new(rx)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(VfsError::Unsupported(
                "directory reader stopped unexpectedly",
            )),
        }
    }

    async fn stat(&self, path: &VPath) -> Result<Metadata> {
        let path_ref = path.as_path();
        let link_metadata = tokio::fs::symlink_metadata(path_ref)
            .await
            .map_err(|error| VfsError::from_io(error, path_ref))?;
        let is_symlink = link_metadata.file_type().is_symlink();
        let resolved = if is_symlink {
            tokio::fs::metadata(path_ref).await.ok()
        } else {
            None
        };
        let effective = resolved.as_ref().unwrap_or(&link_metadata);
        let kind = if effective.is_dir() {
            EntryKind::Dir
        } else if effective.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };

        Ok(Metadata {
            kind,
            size: if kind == EntryKind::Dir {
                0
            } else {
                effective.len()
            },
            modified: effective.modified().ok(),
            is_symlink,
        })
    }

    async fn open_read(&self, path: &VPath) -> Result<Box<dyn AsyncRead + Send + Unpin>> {
        let file = tokio::fs::File::open(path.as_path())
            .await
            .map_err(|error| VfsError::from_io(error, path.as_path()))?;
        Ok(Box::new(file))
    }

    async fn open_write(&self, path: &VPath) -> Result<Box<dyn AsyncWrite + Send + Unpin>> {
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path.as_path())
            .await
            .map_err(|error| VfsError::from_io(error, path.as_path()))?;
        Ok(Box::new(file))
    }

    async fn rename(&self, from: &VPath, to: &VPath) -> Result<()> {
        tokio::fs::rename(from.as_path(), to.as_path())
            .await
            .map_err(|error| VfsError::from_io(error, from.as_path()))
    }

    async fn remove(&self, path: &VPath, mode: RemoveMode) -> Result<()> {
        if mode == RemoveMode::Trash {
            return Err(VfsError::Unsupported(
                "trash is not implemented for the local backend yet",
            ));
        }

        let path_ref = path.as_path();
        let metadata = tokio::fs::symlink_metadata(path_ref)
            .await
            .map_err(|error| VfsError::from_io(error, path_ref))?;
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(path_ref)
                .await
                .map_err(|error| VfsError::from_io(error, path_ref))
        } else {
            tokio::fs::remove_file(path_ref)
                .await
                .map_err(|error| VfsError::from_io(error, path_ref))
        }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_rename: true,
            trash: false,
            symlinks: true,
            streaming_dir: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    async fn collect(mut stream: DirStream) -> Vec<DirEntry> {
        let mut entries = Vec::new();
        while let Some(item) = stream.next().await {
            entries.push(item.expect("directory entry"));
        }
        entries
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[tokio::test]
    async fn lists_entries_with_metadata() {
        let dir = tempdir();
        fs::write(dir.path().join("alpha.txt"), b"hello").unwrap();
        fs::create_dir(dir.path().join("beta")).unwrap();
        fs::write(dir.path().join(".hidden"), b"x").unwrap();

        let fs_backend = LocalFs::new();
        let entries = collect(
            fs_backend
                .read_dir(&VPath::new(dir.path()))
                .await
                .expect("read_dir"),
        )
        .await;

        assert_eq!(entries.len(), 3);
        let alpha = entries
            .iter()
            .find(|entry| entry.name == "alpha.txt")
            .unwrap();
        assert_eq!(alpha.kind, EntryKind::File);
        assert_eq!(alpha.size, Some(5));
        assert!(!alpha.is_hidden);

        let beta = entries.iter().find(|entry| entry.name == "beta").unwrap();
        assert_eq!(beta.kind, EntryKind::Dir);
        assert_eq!(beta.size, None);

        let hidden = entries
            .iter()
            .find(|entry| entry.name == ".hidden")
            .unwrap();
        assert!(hidden.is_hidden);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn marks_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir();
        let path = dir.path().join("run.sh");
        fs::write(&path, b"#!/bin/sh\n").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();

        let entries = collect(
            LocalFs::new()
                .read_dir(&VPath::new(dir.path()))
                .await
                .expect("read_dir"),
        )
        .await;

        let script = entries.iter().find(|entry| entry.name == "run.sh").unwrap();
        assert!(script.is_executable);
    }

    #[tokio::test]
    async fn missing_directory_fails_before_streaming() {
        let fs_backend = LocalFs::new();
        let error = fs_backend
            .read_dir(&VPath::new("/definitely/not/a/real/directory/xyz"))
            .await
            .expect_err("must fail");
        assert!(matches!(error, VfsError::NotFound(_)));
    }

    #[tokio::test]
    async fn stat_reports_kind_and_size() {
        let dir = tempdir();
        let file = dir.path().join("data.bin");
        fs::write(&file, b"1234567890").unwrap();

        let metadata = LocalFs::new().stat(&VPath::new(&file)).await.expect("stat");
        assert_eq!(metadata.kind, EntryKind::File);
        assert_eq!(metadata.size, 10);
        assert!(!metadata.is_symlink);
    }

    #[tokio::test]
    async fn rename_and_remove_round_trip() {
        let dir = tempdir();
        let from = dir.path().join("old.txt");
        let to = dir.path().join("new.txt");
        fs::write(&from, b"x").unwrap();

        let fs_backend = LocalFs::new();
        fs_backend
            .rename(&VPath::new(&from), &VPath::new(&to))
            .await
            .expect("rename");
        assert!(!from.exists());
        assert!(to.exists());

        fs_backend
            .remove(&VPath::new(&to), RemoveMode::Permanent)
            .await
            .expect("remove");
        assert!(!to.exists());
    }

    #[tokio::test]
    async fn trash_is_reported_as_unsupported() {
        let dir = tempdir();
        let path = dir.path().join("keep.txt");
        fs::write(&path, b"x").unwrap();

        let error = LocalFs::new()
            .remove(&VPath::new(&path), RemoveMode::Trash)
            .await
            .expect_err("trash unsupported");
        assert!(matches!(error, VfsError::Unsupported(_)));
        assert!(path.exists());
    }

    #[tokio::test]
    async fn open_write_creates_file() {
        let dir = tempdir();
        let path = PathBuf::from(dir.path()).join("created.txt");
        let mut writer = LocalFs::new()
            .open_write(&VPath::new(&path))
            .await
            .expect("open_write");
        use tokio::io::AsyncWriteExt;
        writer.write_all(b"hi").await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        assert_eq!(fs::read(&path).unwrap(), b"hi");
    }
}
