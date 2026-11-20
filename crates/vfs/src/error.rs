use std::io;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum VfsError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl VfsError {
    pub fn from_io(error: io::Error, path: &Path) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path.display().to_string()),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied(path.display().to_string()),
            io::ErrorKind::NotADirectory => Self::NotADirectory(path.display().to_string()),
            _ => Self::Io(error),
        }
    }
}

pub type Result<T> = std::result::Result<T, VfsError>;
