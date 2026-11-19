use std::cmp::Ordering;
use std::time::{Duration, Instant, SystemTime};

use hwe_vfs::{DirEntry as VfsDirEntry, EntryKind, VPath};

use crate::keymap::Key;

pub const STATUS_TTL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusLevel {
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub level: StatusLevel,
    pub text: String,
    pub expires: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: VPath,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub is_hidden: bool,
    pub is_symlink: bool,
    pub is_executable: bool,
}

impl From<VfsDirEntry> for Entry {
    fn from(entry: VfsDirEntry) -> Self {
        Self {
            name: entry.name,
            path: entry.path,
            kind: entry.kind,
            size: entry.size,
            modified: entry.modified,
            is_hidden: entry.is_hidden,
            is_symlink: entry.is_symlink,
            is_executable: entry.is_executable,
        }
    }
}

#[derive(Debug)]
pub struct TabState {
    pub path: VPath,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl TabState {
    pub fn new(path: VPath) -> Self {
        Self {
            path,
            entries: Vec::new(),
            cursor: 0,
            loading: true,
            error: None,
        }
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) -> bool {
        if self.entries.is_empty() {
            return false;
        }

        let last = self.entries.len() - 1;
        let next = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
        let changed = next != self.cursor;
        self.cursor = next;
        changed
    }

    pub fn jump_to(&mut self, index: usize) -> bool {
        if self.entries.is_empty() {
            return false;
        }

        let next = index.min(self.entries.len() - 1);
        let changed = next != self.cursor;
        self.cursor = next;
        changed
    }

    pub fn jump_top(&mut self) -> bool {
        self.jump_to(0)
    }

    pub fn jump_bottom(&mut self) -> bool {
        self.jump_to(self.entries.len().saturating_sub(1))
    }

    pub fn clamp_cursor(&mut self) {
        if self.entries.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = self.cursor.min(self.entries.len() - 1);
        }
    }
}

#[derive(Debug, Default)]
pub enum Preview {
    #[default]
    None,
    Loading,
    Directory {
        entries: Vec<Entry>,
        truncated: bool,
    },
    Text {
        lines: Vec<String>,
        truncated: bool,
    },
    Binary {
        size: Option<u64>,
    },
    Other,
    Error(String),
}

#[derive(Debug, Default)]
pub struct PreviewState {
    pub path: Option<VPath>,
    pub content: Preview,
}

#[derive(Debug)]
pub struct AppState {
    pub tab: TabState,
    pub status: Option<StatusMessage>,
    pub pending_keys: Vec<Key>,
    pub preview: PreviewState,
}

impl AppState {
    pub fn new(path: VPath) -> Self {
        Self {
            tab: TabState::new(path),
            status: None,
            pending_keys: Vec::new(),
            preview: PreviewState::default(),
        }
    }

    pub fn set_status(&mut self, level: StatusLevel, text: impl Into<String>) {
        self.status = Some(StatusMessage {
            level,
            text: text.into(),
            expires: Some(Instant::now() + STATUS_TTL),
        });
    }

    pub fn expire_status(&mut self) -> bool {
        let expired = self
            .status
            .as_ref()
            .and_then(|status| status.expires)
            .is_some_and(|expires| Instant::now() >= expires);
        if expired {
            self.status = None;
        }
        expired
    }
}

pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        let a_rank = u8::from(a.kind != EntryKind::Dir);
        let b_rank = u8::from(b.kind != EntryKind::Dir);
        a_rank.cmp(&b_rank).then_with(|| name_cmp(&a.name, &b.name))
    });
}

fn name_cmp(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
        .then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            name: name.to_string(),
            path: VPath::new(format!("/tmp/{name}")),
            kind,
            size: None,
            modified: None,
            is_hidden: name.starts_with('.'),
            is_symlink: false,
            is_executable: false,
        }
    }

    fn tab_with(names: &[(&str, EntryKind)]) -> TabState {
        let mut tab = TabState::new(VPath::new("/tmp"));
        tab.entries = names
            .iter()
            .map(|(name, kind)| entry(name, *kind))
            .collect();
        tab.loading = false;
        tab
    }

    #[test]
    fn cursor_moves_and_clamps() {
        let mut tab = tab_with(&[("a", EntryKind::File), ("b", EntryKind::File)]);
        assert!(tab.move_cursor(1));
        assert_eq!(tab.cursor, 1);
        assert!(!tab.move_cursor(1));
        assert_eq!(tab.cursor, 1);
        assert!(tab.move_cursor(-5));
        assert_eq!(tab.cursor, 0);
    }

    #[test]
    fn cursor_moves_are_noops_when_empty() {
        let mut tab = tab_with(&[]);
        assert!(!tab.move_cursor(1));
        assert!(!tab.jump_bottom());
        assert_eq!(tab.cursor, 0);
    }

    #[test]
    fn sort_puts_directories_first_and_ignores_case() {
        let mut entries = vec![
            entry("zeta.txt", EntryKind::File),
            entry("alpha", EntryKind::Dir),
            entry("Beta.txt", EntryKind::File),
            entry("beta", EntryKind::Dir),
        ];
        sort_entries(&mut entries);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "Beta.txt", "zeta.txt"]);
    }

    #[test]
    fn selected_entry_follows_cursor() {
        let mut tab = tab_with(&[("a", EntryKind::File), ("b", EntryKind::Dir)]);
        assert_eq!(tab.selected().unwrap().name, "a");
        tab.move_cursor(1);
        assert_eq!(tab.selected().unwrap().name, "b");
        tab.clamp_cursor();
        assert_eq!(tab.cursor, 1);
    }

    #[test]
    fn status_expires() {
        let mut state = AppState::new(VPath::new("/tmp"));
        state.set_status(StatusLevel::Info, "hello");
        assert!(!state.expire_status());
        state.status.as_mut().unwrap().expires = Some(Instant::now() - Duration::from_secs(1));
        assert!(state.expire_status());
        assert!(state.status.is_none());
    }
}
