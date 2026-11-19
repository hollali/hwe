use std::time::SystemTime;

use crate::VPath;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Other,
}

#[derive(Clone, Debug)]
pub struct DirEntry {
    pub name: String,
    pub path: VPath,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub is_hidden: bool,
    pub is_symlink: bool,
    pub is_executable: bool,
    pub mode: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub is_symlink: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoveMode {
    Trash,
    Permanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub atomic_rename: bool,
    pub trash: bool,
    pub symlinks: bool,
    pub streaming_dir: bool,
}
