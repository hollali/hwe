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
    pub mode: Option<u32>,
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
            mode: entry.mode,
        }
    }
}

#[derive(Debug)]
pub struct TabState {
    pub id: u64,
    pub load_id: u64,
    pub path: VPath,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl TabState {
    pub fn new(path: VPath) -> Self {
        Self {
            id: 0,
            load_id: 0,
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
    pub scroll: usize,
    pub focus: bool,
}

impl PreviewState {
    pub fn content_len(&self) -> usize {
        match &self.content {
            Preview::Directory { entries, .. } => entries.len(),
            Preview::Text { lines, .. } => lines.len(),
            _ => 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct ParentState {
    pub path: Option<VPath>,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PaletteMatch {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct PaletteState {
    pub query: String,
    pub selected: usize,
    pub matches: Vec<PaletteMatch>,
}

#[derive(Debug)]
pub struct AppState {
    pub tabs: Vec<TabState>,
    pub active: usize,
    pub status: Option<StatusMessage>,
    pub pending_keys: Vec<Key>,
    pub preview: PreviewState,
    pub parent: ParentState,
    pub help: bool,
    pub palette: Option<PaletteState>,
}

impl AppState {
    pub fn new(path: VPath) -> Self {
        let mut tab = TabState::new(path);
        tab.id = 1;
        Self {
            tabs: vec![tab],
            active: 0,
            status: None,
            pending_keys: Vec::new(),
            preview: PreviewState::default(),
            parent: ParentState::default(),
            help: false,
            palette: None,
        }
    }

    pub fn tab(&self) -> &TabState {
        &self.tabs[self.active]
    }

    pub fn tab_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active]
    }

    pub fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    pub fn open_tab(&mut self) -> usize {
        let path = self.tab().path.clone();
        let id = self.tabs.iter().map(|tab| tab.id).max().unwrap_or(0) + 1;
        let mut tab = TabState::new(path);
        tab.id = id;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.active
    }

    pub fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        self.tabs.remove(self.active);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        true
    }

    pub fn next_tab(&mut self) {
        self.active = (self.active + 1) % self.tabs.len();
    }

    pub fn previous_tab(&mut self) {
        self.active = if self.active == 0 {
            self.tabs.len() - 1
        } else {
            self.active - 1
        };
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
            mode: None,
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
    fn tabs_open_switch_and_close() {
        let mut state = AppState::new(VPath::new("/tmp"));
        assert_eq!(state.tabs.len(), 1);
        let first = state.tab().id;

        state.open_tab();
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(state.active, 1);
        assert_ne!(state.tab().id, first);

        state.next_tab();
        assert_eq!(state.active, 0);
        state.previous_tab();
        assert_eq!(state.active, 1);

        assert!(state.close_active_tab());
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active, 0);
        assert!(!state.close_active_tab());
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
