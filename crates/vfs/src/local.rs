use std::fs;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::{
    Capabilities, DirEntry, DirStream, EntryKind, Metadata, RemoveMode, Result, VPath, Vfs,
    VfsError,
};

#[derive(Clone, Debug, Default)]
pub struct LocalFs {
    trash_root: Option<PathBuf>,
}

impl LocalFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_trash_root(path: impl Into<PathBuf>) -> Self {
        Self {
            trash_root: Some(path.into()),
        }
    }

    fn trash_root(&self) -> Option<PathBuf> {
        if let Some(root) = &self.trash_root {
            return Some(root.clone());
        }

        #[cfg(target_os = "linux")]
        {
            let data_home = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
                });
            data_home.map(|home| home.join("Trash"))
        }

        #[cfg(target_os = "macos")]
        {
            return std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Trash"));
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            None
        }
    }

    async fn remove_permanent(&self, path: &VPath) -> Result<()> {
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

    async fn trash(&self, path: &VPath) -> Result<()> {
        let Some(root) = self.trash_root() else {
            return Err(VfsError::Unsupported(
                "trash is not available on this platform",
            ));
        };

        let path_ref = path.as_path();
        let name = path_ref
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".to_string());

        #[cfg(target_os = "linux")]
        {
            let files_dir = root.join("files");
            let info_dir = root.join("info");
            tokio::fs::create_dir_all(&files_dir)
                .await
                .map_err(|error| VfsError::from_io(error, &files_dir))?;
            tokio::fs::create_dir_all(&info_dir)
                .await
                .map_err(|error| VfsError::from_io(error, &info_dir))?;

            let mut suffix = 0u32;
            let (destination, info_path) = loop {
                let candidate = if suffix == 0 {
                    name.clone()
                } else {
                    format!("{name}.{suffix}")
                };
                let destination = files_dir.join(&candidate);
                let info_path = info_dir.join(format!("{candidate}.trashinfo"));
                if path_available(&destination).await && path_available(&info_path).await {
                    break (destination, info_path);
                }
                suffix += 1;
            };

            let source = absolute_path(path_ref);
            let info = format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                percent_encode_path(&source),
                utc_timestamp()
            );
            tokio::fs::write(&info_path, info)
                .await
                .map_err(|error| VfsError::from_io(error, &info_path))?;
            if let Err(error) = move_into(&source, &destination, self).await {
                let _ = tokio::fs::remove_file(&info_path).await;
                return Err(error);
            }
            Ok(())
        }

        #[cfg(target_os = "macos")]
        {
            tokio::fs::create_dir_all(&root)
                .await
                .map_err(|error| VfsError::from_io(error, &root))?;
            let mut suffix = 0u32;
            let destination = loop {
                let candidate = if suffix == 0 {
                    name.clone()
                } else {
                    format!("{name}.{suffix}")
                };
                let destination = root.join(candidate);
                if !destination.exists() {
                    break destination;
                }
                suffix += 1;
            };
            let source = absolute_path(path_ref);
            move_into(&source, &destination, self).await
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (root, name);
            Err(VfsError::Unsupported(
                "trash is not available on this platform",
            ))
        }
    }
}

async fn path_available(path: &Path) -> bool {
    match tokio::fs::symlink_metadata(path).await {
        Ok(_) => false,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn percent_encode_path(path: &Path) -> String {
    #[cfg(unix)]
    let bytes: Vec<u8> = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().as_bytes().to_vec();

    let mut encoded = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

async fn move_into(source: &Path, destination: &Path, backend: &LocalFs) -> Result<()> {
    match tokio::fs::rename(source, destination).await {
        Ok(()) => Ok(()),
        Err(_) => {
            let from = VPath::new(source);
            let to = VPath::new(destination);
            backend.copy(&from, &to).await?;
            backend.remove_permanent(&from).await
        }
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
    let link_metadata = raw.metadata().ok();
    let target_metadata = if is_symlink {
        fs::metadata(path.as_path()).ok()
    } else {
        None
    };
    let metadata = target_metadata.as_ref().or(link_metadata.as_ref());
    let kind = match metadata {
        Some(metadata) if metadata.is_dir() => EntryKind::Dir,
        Some(metadata) if metadata.is_file() => EntryKind::File,
        _ => EntryKind::Other,
    };
    let size = match metadata {
        Some(metadata) if !metadata.is_dir() => Some(metadata.len()),
        _ => None,
    };
    #[cfg(unix)]
    let is_executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_some_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    #[cfg(windows)]
    let is_executable = {
        let extension = name.rsplit_once('.').map(|(_, extension)| extension);
        extension.is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "bat" | "cmd" | "com" | "ps1" | "msi"
            )
        })
    };
    #[cfg(not(any(unix, windows)))]
    let is_executable = false;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::MetadataExt;
        metadata.map(|meta| meta.mode())
    };
    #[cfg(not(unix))]
    let mode = None;

    let modified = metadata.and_then(|metadata| metadata.modified().ok());
    let is_hidden = name.starts_with('.');
    #[cfg(windows)]
    let is_hidden = is_hidden
        || metadata.is_some_and(|metadata| {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x2 != 0
        });

    Ok(DirEntry {
        name,
        path,
        kind,
        size,
        modified,
        is_hidden,
        is_symlink,
        is_executable,
        mode,
    })
}

async fn copy_symlink(src: &Path, dst: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let target = tokio::fs::read_link(src)
            .await
            .map_err(|error| VfsError::from_io(error, src))?;
        tokio::fs::symlink(target, dst)
            .await
            .map_err(|error| VfsError::from_io(error, dst))
    }
    #[cfg(not(unix))]
    {
        tokio::fs::copy(src, dst)
            .await
            .map(|_| ())
            .map_err(|error| VfsError::from_io(error, src))
    }
}

#[cfg(target_os = "linux")]
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[cfg(target_os = "linux")]
fn utc_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        remainder / 3_600,
        (remainder % 3_600) / 60,
        remainder % 60
    )
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

    async fn create_dir(&self, path: &VPath) -> Result<()> {
        tokio::fs::create_dir_all(path.as_path())
            .await
            .map_err(|error| VfsError::from_io(error, path.as_path()))
    }

    async fn create_file(&self, path: &VPath) -> Result<()> {
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_path())
            .await
            .map(|_| ())
            .map_err(|error| VfsError::from_io(error, path.as_path()))
    }

    async fn copy(&self, from: &VPath, to: &VPath) -> Result<()> {
        let from_path = from.as_path();
        let to_path = to.as_path();
        if to_path.starts_with(from_path) && to_path != from_path {
            return Err(VfsError::InvalidInput(format!(
                "cannot copy {} into itself",
                from_path.display()
            )));
        }

        let mut stack = vec![(from_path.to_path_buf(), to_path.to_path_buf())];
        while let Some((src, dst)) = stack.pop() {
            let metadata = tokio::fs::symlink_metadata(&src)
                .await
                .map_err(|error| VfsError::from_io(error, &src))?;

            if metadata.file_type().is_symlink() {
                copy_symlink(&src, &dst).await?;
            } else if metadata.is_dir() {
                tokio::fs::create_dir_all(&dst)
                    .await
                    .map_err(|error| VfsError::from_io(error, &dst))?;
                let mut reader = tokio::fs::read_dir(&src)
                    .await
                    .map_err(|error| VfsError::from_io(error, &src))?;
                while let Some(entry) = reader
                    .next_entry()
                    .await
                    .map_err(|error| VfsError::from_io(error, &src))?
                {
                    stack.push((entry.path(), dst.join(entry.file_name())));
                }
            } else {
                tokio::fs::copy(&src, &dst)
                    .await
                    .map_err(|error| VfsError::from_io(error, &src))?;
            }
        }

        Ok(())
    }

    async fn rename(&self, from: &VPath, to: &VPath) -> Result<()> {
        tokio::fs::rename(from.as_path(), to.as_path())
            .await
            .map_err(|error| VfsError::from_io(error, from.as_path()))
    }

    async fn remove(&self, path: &VPath, mode: RemoveMode) -> Result<()> {
        if mode == RemoveMode::Trash {
            return self.trash(path).await;
        }

        self.remove_permanent(path).await
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            atomic_rename: true,
            trash: cfg!(any(target_os = "linux", target_os = "macos")),
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

    #[cfg(unix)]
    #[tokio::test]
    async fn follows_symlinked_directories() {
        let dir = tempdir();
        let real = dir.path().join("real");
        let link = dir.path().join("link");
        fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let entries = collect(
            LocalFs::new()
                .read_dir(&VPath::new(dir.path()))
                .await
                .expect("read_dir"),
        )
        .await;

        let link = entries.iter().find(|entry| entry.name == "link").unwrap();
        assert_eq!(link.kind, EntryKind::Dir);
        assert!(link.is_symlink);
    }

    #[tokio::test]
    async fn create_dir_and_file() {
        let dir = tempdir();
        let nested = dir.path().join("a/b");
        let file = dir.path().join("note.txt");

        let fs_backend = LocalFs::new();
        fs_backend
            .create_dir(&VPath::new(&nested))
            .await
            .expect("create_dir");
        fs_backend
            .create_file(&VPath::new(&file))
            .await
            .expect("create_file");
        assert!(nested.is_dir());
        assert!(file.is_file());
    }

    #[tokio::test]
    async fn copies_files_and_directories() {
        let dir = tempdir();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("a.txt"), b"a").unwrap();
        fs::write(source.join("nested/b.txt"), b"b").unwrap();

        LocalFs::new()
            .copy(&VPath::new(&source), &VPath::new(&destination))
            .await
            .expect("copy");

        assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(destination.join("nested/b.txt")).unwrap(), b"b");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn trash_moves_file_and_writes_info() {
        let dir = tempdir();
        let trash = tempdir();
        let path = dir.path().join("trash-me.txt");
        fs::write(&path, b"x").unwrap();

        LocalFs::with_trash_root(trash.path())
            .remove(&VPath::new(&path), RemoveMode::Trash)
            .await
            .expect("trash");

        assert!(!path.exists());
        assert!(trash.path().join("files/trash-me.txt").is_file());
        let info = fs::read_to_string(trash.path().join("info/trash-me.txt.trashinfo")).unwrap();
        assert!(info.contains("[Trash Info]"));
        assert!(info.contains("Path="));
    }

    #[tokio::test]
    async fn create_file_does_not_truncate_existing_files() {
        let dir = tempdir();
        let path = dir.path().join("existing.txt");
        fs::write(&path, b"keep me").unwrap();

        LocalFs::new()
            .create_file(&VPath::new(&path))
            .await
            .expect("create_file");

        assert_eq!(fs::read(&path).unwrap(), b"keep me");
    }

    #[cfg(unix)]
    #[test]
    fn trash_paths_are_percent_encoded() {
        let path = std::path::Path::new("/tmp/a b#c.txt");
        assert_eq!(super::percent_encode_path(path), "/tmp/a%20b%23c.txt");
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
