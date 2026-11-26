use std::cmp::Ordering;
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime};

use hwe_vfs::{DirEntry as VfsDirEntry, EntryKind, RemoveMode, VPath};

use crate::keymap::Key;

pub const STATUS_TTL: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortMode {
    #[default]
    Name,
    Size,
    Modified,
    Extension,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::Modified,
            Self::Modified => Self::Extension,
            Self::Extension => Self::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Modified => "modified",
            Self::Extension => "extension",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "name" => Some(Self::Name),
            "size" => Some(Self::Size),
            "modified" | "date" | "time" => Some(Self::Modified),
            "extension" | "ext" | "type" => Some(Self::Extension),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SortSettings {
    pub mode: SortMode,
    pub dirs_first: bool,
    pub reverse: bool,
}

impl Default for SortSettings {
    fn default() -> Self {
        Self {
            mode: SortMode::Name,
            dirs_first: true,
            reverse: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub show_hidden: bool,
    pub sort: SortSettings,
}

#[derive(Clone, Debug)]
pub enum ConfirmAction {
    Delete { paths: Vec<VPath>, mode: RemoveMode },
}

#[derive(Clone, Debug)]
pub enum PromptKind {
    Mkdir,
    Touch,
    Rename { target: VPath },
    Filter,
    Search,
    Confirm { action: ConfirmAction },
}

#[derive(Clone, Debug)]
pub struct PromptState {
    pub kind: PromptKind,
    pub title: String,
    pub input: String,
    pub hint: String,
}

impl PromptState {
    pub fn text(kind: PromptKind, title: impl Into<String>, input: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            input: input.into(),
            hint: "Enter confirms · Esc cancels".to_string(),
        }
    }

    pub fn confirm(action: ConfirmAction, title: impl Into<String>) -> Self {
        Self {
            kind: PromptKind::Confirm { action },
            title: title.into(),
            input: String::new(),
            hint: "Enter/y confirms · Esc/n cancels".to_string(),
        }
    }

    pub fn is_confirm(&self) -> bool {
        matches!(self.kind, PromptKind::Confirm { .. })
    }
}

#[derive(Clone, Debug)]
pub struct Clipboard {
    pub paths: Vec<VPath>,
    pub cut: bool,
}

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

#[derive(Clone, Debug)]
pub struct SearchState {
    pub id: u64,
    pub query: String,
    pub root: VPath,
    pub previous_entries: Vec<Entry>,
    pub previous_cursor: usize,
    pub previous_filter: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub matches: usize,
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
    pub marks: HashSet<VPath>,
    pub filter: Option<String>,
    pub search: Option<SearchState>,
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
            marks: HashSet::new(),
            filter: None,
            search: None,
        }
    }

    pub fn visible_indices(&self, settings: &Settings) -> Vec<usize> {
        let query = self.filter.as_ref().map(|query| query.to_ascii_lowercase());
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                (settings.show_hidden || !entry.is_hidden)
                    && query
                        .as_ref()
                        .is_none_or(|query| entry.name.to_ascii_lowercase().contains(query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub fn visible_entries(&self, settings: &Settings) -> Vec<&Entry> {
        self.visible_indices(settings)
            .into_iter()
            .filter_map(|index| self.entries.get(index))
            .collect()
    }

    pub fn selected(&self, settings: &Settings) -> Option<&Entry> {
        let index = *self.visible_indices(settings).get(self.cursor)?;
        self.entries.get(index)
    }

    pub fn move_cursor(&mut self, delta: isize, settings: &Settings) -> bool {
        let len = self.visible_indices(settings).len();
        if len == 0 {
            return false;
        }

        let last = len - 1;
        let next = (self.cursor as isize + delta).clamp(0, last as isize) as usize;
        let changed = next != self.cursor;
        self.cursor = next;
        changed
    }

    pub fn jump_to(&mut self, index: usize, settings: &Settings) -> bool {
        let len = self.visible_indices(settings).len();
        if len == 0 {
            return false;
        }

        let next = index.min(len - 1);
        let changed = next != self.cursor;
        self.cursor = next;
        changed
    }

    pub fn jump_top(&mut self, settings: &Settings) -> bool {
        self.jump_to(0, settings)
    }

    pub fn jump_bottom(&mut self, settings: &Settings) -> bool {
        let last = self.visible_indices(settings).len().saturating_sub(1);
        self.jump_to(last, settings)
    }

    pub fn clamp_cursor(&mut self, settings: &Settings) {
        let last = self.visible_indices(settings).len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
    }

    pub fn cursor_for_name(&self, settings: &Settings, name: &str) -> Option<usize> {
        self.visible_indices(settings)
            .into_iter()
            .position(|index| {
                self.entries
                    .get(index)
                    .is_some_and(|entry| entry.name == name)
            })
    }

    pub fn cursor_for_path(&self, settings: &Settings, path: &VPath) -> Option<usize> {
        self.visible_indices(settings)
            .into_iter()
            .position(|index| {
                self.entries
                    .get(index)
                    .is_some_and(|entry| &entry.path == path)
            })
    }

    pub fn is_marked(&self, entry: &Entry) -> bool {
        self.marks.contains(&entry.path)
    }

    pub fn toggle_mark(&mut self, settings: &Settings) -> bool {
        let Some(path) = self.selected(settings).map(|entry| entry.path.clone()) else {
            return false;
        };
        if !self.marks.remove(&path) {
            self.marks.insert(path);
        }
        true
    }

    pub fn select_all(&mut self, settings: &Settings) {
        self.marks = self
            .visible_entries(settings)
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect();
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

impl PaletteState {
    pub fn command_query(&self) -> (&str, Vec<String>) {
        let query = self.query.trim();
        match query.split_once(char::is_whitespace) {
            Some((name, args)) => (name, args.split_whitespace().map(str::to_string).collect()),
            None => (query, Vec::new()),
        }
    }
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
    pub prompt: Option<PromptState>,
    pub settings: Settings,
    pub clipboard: Option<Clipboard>,
    pub load_state: Option<(u64, u64)>,
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
            prompt: None,
            settings: Settings::default(),
            clipboard: None,
            load_state: None,
        }
    }

    pub fn prompt_mut(&mut self) -> Option<&mut PromptState> {
        self.prompt.as_mut()
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

    pub fn selected(&self) -> Option<&Entry> {
        self.tab().selected(&self.settings)
    }

    pub fn visible_entries(&self) -> Vec<&Entry> {
        self.tab().visible_entries(&self.settings)
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

pub fn sort_entries(entries: &mut [Entry], settings: SortSettings) {
    entries.sort_by(|a, b| {
        if settings.dirs_first {
            let a_rank = u8::from(a.kind != EntryKind::Dir);
            let b_rank = u8::from(b.kind != EntryKind::Dir);
            let rank_order = a_rank.cmp(&b_rank);
            if rank_order != Ordering::Equal {
                return rank_order;
            }
        }

        let primary = match settings.mode {
            SortMode::Name => name_cmp(&a.name, &b.name),
            SortMode::Size => a
                .size
                .unwrap_or(0)
                .cmp(&b.size.unwrap_or(0))
                .then_with(|| name_cmp(&a.name, &b.name)),
            SortMode::Modified => a
                .modified
                .cmp(&b.modified)
                .then_with(|| name_cmp(&a.name, &b.name)),
            SortMode::Extension => {
                let a_ext = extension_of(&a.name);
                let b_ext = extension_of(&b.name);
                a_ext.cmp(&b_ext).then_with(|| name_cmp(&a.name, &b.name))
            }
        };

        if settings.reverse {
            primary.reverse()
        } else {
            primary
        }
    });
}

fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
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
        let settings = Settings::default();
        let mut tab = tab_with(&[("a", EntryKind::File), ("b", EntryKind::File)]);
        assert!(tab.move_cursor(1, &settings));
        assert_eq!(tab.cursor, 1);
        assert!(!tab.move_cursor(1, &settings));
        assert_eq!(tab.cursor, 1);
        assert!(tab.move_cursor(-5, &settings));
        assert_eq!(tab.cursor, 0);
    }

    #[test]
    fn cursor_moves_are_noops_when_empty() {
        let settings = Settings::default();
        let mut tab = tab_with(&[]);
        assert!(!tab.move_cursor(1, &settings));
        assert!(!tab.jump_bottom(&settings));
        assert_eq!(tab.cursor, 0);
    }

    #[test]
    fn hidden_and_filtered_entries_are_not_visible() {
        let mut settings = Settings::default();
        let tab = tab_with(&[
            ("a.txt", EntryKind::File),
            (".hidden", EntryKind::File),
            ("b.md", EntryKind::File),
        ]);

        let names: Vec<&str> = tab
            .visible_entries(&settings)
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["a.txt", "b.md"]);

        settings.show_hidden = true;
        assert_eq!(tab.visible_entries(&settings).len(), 3);

        let mut filtered = tab_with(&[
            ("alpha.txt", EntryKind::File),
            ("beta.md", EntryKind::File),
            ("gamma.txt", EntryKind::File),
        ]);
        filtered.filter = Some("TXT".into());
        let names: Vec<&str> = filtered
            .visible_entries(&settings)
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, ["alpha.txt", "gamma.txt"]);
        assert!(filtered.move_cursor(1, &settings));
        assert_eq!(filtered.selected(&settings).unwrap().name, "gamma.txt");
        assert!(filtered.move_cursor(-1, &settings));
        assert!(filtered.jump_bottom(&settings));
        assert_eq!(filtered.selected(&settings).unwrap().name, "gamma.txt");
    }

    #[test]
    fn marks_follow_paths_when_entries_are_reordered() {
        let settings = Settings::default();
        let mut tab = tab_with(&[("a", EntryKind::File), ("b", EntryKind::File)]);
        assert!(tab.toggle_mark(&settings));
        assert_eq!(tab.marks.len(), 1);
        assert!(tab.is_marked(&tab.entries[0]));

        tab.entries.reverse();
        assert!(tab.is_marked(&tab.entries[1]));
        assert!(!tab.is_marked(&tab.entries[0]));
    }

    #[test]
    fn sort_puts_directories_first_and_ignores_case() {
        let mut entries = vec![
            entry("zeta.txt", EntryKind::File),
            entry("alpha", EntryKind::Dir),
            entry("Beta.txt", EntryKind::File),
            entry("beta", EntryKind::Dir),
        ];
        sort_entries(&mut entries, SortSettings::default());
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta", "Beta.txt", "zeta.txt"]);
    }

    #[test]
    fn reverse_sort_keeps_directories_first() {
        let mut entries = vec![
            entry("zeta.txt", EntryKind::File),
            entry("alpha", EntryKind::Dir),
            entry("beta.txt", EntryKind::File),
        ];
        sort_entries(
            &mut entries,
            SortSettings {
                reverse: true,
                ..SortSettings::default()
            },
        );
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["alpha", "zeta.txt", "beta.txt"]);
    }

    #[test]
    fn selected_entry_follows_cursor() {
        let settings = Settings::default();
        let mut tab = tab_with(&[("a", EntryKind::File), ("b", EntryKind::Dir)]);
        assert_eq!(tab.selected(&settings).unwrap().name, "a");
        tab.move_cursor(1, &settings);
        assert_eq!(tab.selected(&settings).unwrap().name, "b");
        tab.clamp_cursor(&settings);
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
    fn palette_query_splits_command_arguments() {
        let palette = PaletteState {
            query: "  app.filter   note.txt  ".into(),
            ..PaletteState::default()
        };
        let (name, args) = palette.command_query();
        assert_eq!(name, "app.filter");
        assert_eq!(args, ["note.txt"]);

        let palette = PaletteState {
            query: "filter".into(),
            ..PaletteState::default()
        };
        let (name, args) = palette.command_query();
        assert_eq!(name, "filter");
        assert!(args.is_empty());
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
