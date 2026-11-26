use std::collections::HashSet;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use hwe_vfs::{DirEntry as VfsDirEntry, EntryKind, RemoveMode, VPath, Vfs};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::state::{
    AppState, Clipboard, Entry, PaletteMatch, PaletteState, Preview, PreviewState, PromptKind,
    PromptState, SearchState, Settings, StatusLevel, sort_entries,
};

use crate::command::{Args, CommandContext, CommandRegistry};
use crate::commands;
use crate::effect::Effect;
use crate::event::AppEvent;
use crate::keymap::{Key, Keymap, Resolution};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const MAX_PREVIEW_ENTRIES: usize = 1000;
const MAX_PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_PREVIEW_LINES: usize = 400;

pub trait Renderer {
    fn draw(&mut self, state: &AppState) -> std::io::Result<()>;

    fn cycle_theme(&mut self) {}

    fn toggle_icons(&mut self) {}
}

enum DirUpdate {
    Ready {
        tab_id: u64,
        load_id: u64,
        path: VPath,
    },
    Entry {
        tab_id: u64,
        load_id: u64,
        entry: VfsDirEntry,
    },
    Finished {
        tab_id: u64,
        load_id: u64,
    },
    Failed {
        tab_id: u64,
        load_id: u64,
        error: String,
    },
    Notice {
        tab_id: u64,
        load_id: u64,
        level: StatusLevel,
        text: String,
    },
    ClipboardFinished {
        clipboard_id: u64,
        success: bool,
    },
}

struct PreviewUpdate {
    id: u64,
    content: Preview,
}

struct ParentUpdate {
    id: u64,
    entries: Vec<Entry>,
    error: Option<String>,
}

enum SearchUpdate {
    Entry {
        tab_id: u64,
        search_id: u64,
        entry: Entry,
    },
    Finished {
        tab_id: u64,
        search_id: u64,
    },
    Failed {
        tab_id: u64,
        search_id: u64,
        error: String,
    },
    Notice {
        tab_id: u64,
        search_id: u64,
        text: String,
    },
}

pub struct App {
    state: AppState,
    registry: CommandRegistry,
    keymap: Keymap,
    vfs: Arc<dyn Vfs>,
    events: mpsc::UnboundedReceiver<AppEvent>,
    dir_tx: mpsc::Sender<DirUpdate>,
    dir_rx: mpsc::Receiver<DirUpdate>,
    load_handle: Option<JoinHandle<()>>,
    load_retries: u32,
    preview_tx: mpsc::Sender<PreviewUpdate>,
    preview_rx: mpsc::Receiver<PreviewUpdate>,
    preview_handle: Option<JoinHandle<()>>,
    preview_id: u64,
    parent_tx: mpsc::Sender<ParentUpdate>,
    parent_rx: mpsc::Receiver<ParentUpdate>,
    parent_handle: Option<JoinHandle<()>>,
    parent_id: u64,
    search_tx: mpsc::Sender<SearchUpdate>,
    search_rx: mpsc::Receiver<SearchUpdate>,
    search_handle: Option<JoinHandle<()>>,
    search_owner: Option<u64>,
    search_id: u64,
    pending_select: Option<(u64, String)>,
    op_handles: Vec<JoinHandle<()>>,
    clipboard_id: u64,
    cycle_theme: bool,
    toggle_icons: bool,
    dirty: bool,
    quit: bool,
}

impl App {
    pub fn new(
        vfs: Arc<dyn Vfs>,
        start: VPath,
        events: mpsc::UnboundedReceiver<AppEvent>,
        settings: Settings,
    ) -> Self {
        let (dir_tx, dir_rx) = mpsc::channel(512);
        let (preview_tx, preview_rx) = mpsc::channel(64);
        let (parent_tx, parent_rx) = mpsc::channel(16);
        let (search_tx, search_rx) = mpsc::channel(1024);
        Self {
            state: {
                let mut state = AppState::new(start);
                state.settings = settings;
                state
            },
            registry: commands::registry(),
            keymap: Keymap::vim(),
            vfs,
            events,
            dir_tx,
            dir_rx,
            load_handle: None,
            load_retries: 0,
            preview_tx,
            preview_rx,
            preview_handle: None,
            preview_id: 0,
            parent_tx,
            parent_rx,
            parent_handle: None,
            parent_id: 0,
            search_tx,
            search_rx,
            search_handle: None,
            search_owner: None,
            search_id: 0,
            pending_select: None,
            op_handles: Vec::new(),
            clipboard_id: 0,
            cycle_theme: false,
            toggle_icons: false,
            dirty: true,
            quit: false,
        }
    }

    pub async fn run(mut self, renderer: &mut dyn Renderer) -> std::io::Result<Settings> {
        let start = self.state.tab().path.clone();
        self.start_load(start, None);

        let mut ticker = tokio::time::interval(FRAME_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if self.state.expire_status()
                        || self.state.tab().loading
                        || self
                            .state
                            .tab()
                            .search
                            .as_ref()
                            .is_some_and(|search| search.loading)
                    {
                        self.dirty = true;
                    }
                    if std::mem::take(&mut self.cycle_theme) {
                        renderer.cycle_theme();
                        self.dirty = true;
                    }
                    if std::mem::take(&mut self.toggle_icons) {
                        renderer.toggle_icons();
                        self.dirty = true;
                    }
                    self.sync_preview();
                    if self.dirty {
                        renderer.draw(&self.state)?;
                        self.dirty = false;
                    }
                }
                maybe_event = self.events.recv() => {
                    let Some(event) = maybe_event else {
                        break;
                    };
                    self.handle_event(event);
                    for _ in 0..512 {
                        match self.events.try_recv() {
                            Ok(event) => self.handle_event(event),
                            Err(_) => break,
                        }
                    }
                }
                maybe_preview = self.preview_rx.recv() => {
                    let Some(update) = maybe_preview else {
                        break;
                    };
                    self.apply_preview_update(update);
                    for _ in 0..64 {
                        match self.preview_rx.try_recv() {
                            Ok(update) => self.apply_preview_update(update),
                            Err(_) => break,
                        }
                    }
                }
                maybe_parent = self.parent_rx.recv() => {
                    let Some(update) = maybe_parent else {
                        break;
                    };
                    self.apply_parent_update(update);
                    for _ in 0..16 {
                        match self.parent_rx.try_recv() {
                            Ok(update) => self.apply_parent_update(update),
                            Err(_) => break,
                        }
                    }
                }
                maybe_update = self.dir_rx.recv() => {
                    let Some(update) = maybe_update else {
                        break;
                    };
                    self.apply_dir_update(update);
                    for _ in 0..1024 {
                        match self.dir_rx.try_recv() {
                            Ok(update) => self.apply_dir_update(update),
                            Err(_) => break,
                        }
                    }
                }
                maybe_search = self.search_rx.recv() => {
                    let Some(update) = maybe_search else {
                        break;
                    };
                    self.apply_search_update(update);
                    for _ in 0..1024 {
                        match self.search_rx.try_recv() {
                            Ok(update) => self.apply_search_update(update),
                            Err(_) => break,
                        }
                    }
                }
            }

            if self.quit {
                break;
            }
        }

        if let Some(handle) = self.load_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.preview_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.parent_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }
        for handle in self.op_handles.drain(..) {
            handle.abort();
        }

        Ok(self.state.settings)
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize { .. } => self.dirty = true,
            AppEvent::Paste(text) => {
                let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
                if let Some(prompt) = &mut self.state.prompt {
                    prompt.input.push_str(&text);
                } else if let Some(palette) = &mut self.state.palette {
                    palette.query.push_str(&text);
                    palette.selected = 0;
                    self.refresh_palette();
                }
                self.dirty = true;
            }
        }
    }

    fn handle_key(&mut self, key: Key) {
        if self.state.help {
            self.state.help = false;
            self.dirty = true;
            if matches!(key, Key::Esc | Key::Char('?')) {
                return;
            }
        }

        if self.state.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        if self.state.palette.is_some() {
            self.handle_palette_key(key);
            return;
        }

        if key == Key::Char(':') {
            self.open_palette();
            return;
        }

        if self.state.preview.focus && self.handle_preview_key(key) {
            return;
        }

        match self.keymap.resolve(key) {
            Resolution::Pending => {
                self.state.pending_keys = self.keymap.pending().to_vec();
                self.dirty = true;
            }
            Resolution::Unbound => {
                if !self.state.pending_keys.is_empty() {
                    self.clear_pending();
                    self.dirty = true;
                }
            }
            Resolution::Command(name) => {
                self.clear_pending();
                self.dispatch(name);
            }
        }
    }

    fn clear_pending(&mut self) {
        self.keymap.clear_pending();
        self.state.pending_keys.clear();
    }

    fn handle_preview_key(&mut self, key: Key) -> bool {
        self.clear_pending();
        match key {
            Key::Char('j') | Key::Down => {
                self.scroll_preview(1);
                true
            }
            Key::Char('k') | Key::Up => {
                self.scroll_preview(-1);
                true
            }
            Key::PageDown => {
                self.scroll_preview(10);
                true
            }
            Key::PageUp => {
                self.scroll_preview(-10);
                true
            }
            Key::Tab | Key::Esc => {
                self.state.preview.focus = false;
                self.dirty = true;
                true
            }
            _ => {
                self.state.preview.focus = false;
                self.dirty = true;
                false
            }
        }
    }

    fn scroll_preview(&mut self, delta: i32) {
        let length = self.state.preview.content_len();
        if length == 0 {
            return;
        }

        let current = self.state.preview.scroll;
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            current.saturating_add(delta as usize)
        };
        self.state.preview.scroll = next.min(length.saturating_sub(1));
        self.dirty = true;
    }

    fn selected_paths(&self) -> Vec<VPath> {
        let tab = self.state.tab();
        let marked: Vec<VPath> = tab
            .entries
            .iter()
            .filter(|entry| tab.is_marked(entry))
            .map(|entry| entry.path.clone())
            .collect();
        if marked.is_empty() {
            self.state
                .selected()
                .map(|entry| entry.path.clone())
                .into_iter()
                .collect()
        } else {
            marked
        }
    }

    fn resort_all(&mut self) {
        let settings = self.state.settings;
        for tab in &mut self.state.tabs {
            sort_entries(&mut tab.entries, settings.sort);
            tab.clamp_cursor(&settings);
        }
        self.dirty = true;
    }

    fn refresh_view(&mut self) {
        let settings = self.state.settings;
        for tab in &mut self.state.tabs {
            tab.clamp_cursor(&settings);
        }
        let focus = self.state.preview.focus;
        self.invalidate_preview();
        self.state.preview.focus = focus;
        self.dirty = true;
    }

    fn begin_operation(&mut self) -> (VPath, u64, u64) {
        let path = self.state.tab().path.clone();
        let tab_id = self.state.tab().id;
        self.abort_search_for_tab(tab_id);
        self.abort_directory_load();
        let load_id = {
            let tab = self.state.tab_mut();
            tab.load_id = tab.load_id.wrapping_add(1);
            tab.loading = true;
            tab.error = None;
            tab.entries.clear();
            tab.cursor = 0;
            tab.marks.clear();
            tab.search = None;
            tab.load_id
        };
        self.state.load_state = Some((tab_id, load_id));
        self.start_parent_load(&path);
        self.dirty = true;
        (path, tab_id, load_id)
    }

    fn copy_selection(&mut self, cut: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.state.clipboard = Some(Clipboard { paths, cut });
        self.clipboard_id = self.clipboard_id.wrapping_add(1);
        self.state
            .set_status(StatusLevel::Info, if cut { "cut" } else { "copied" });
        self.dirty = true;
    }

    fn paste_clipboard(&mut self) {
        let Some(clipboard) = self.state.clipboard.clone() else {
            self.state
                .set_status(StatusLevel::Info, "clipboard is empty");
            self.dirty = true;
            return;
        };
        let (destination_dir, tab_id, load_id) = self.begin_operation();
        let cut = clipboard.cut;
        self.clipboard_id = self.clipboard_id.wrapping_add(1);
        let clipboard_id = self.clipboard_id;

        let vfs = Arc::clone(&self.vfs);
        let paths = clipboard.paths;
        let tx = self.dir_tx.clone();
        let handle = tokio::spawn(async move {
            let mut failures = Vec::new();
            let mut done = 0;
            for source in &paths {
                let Some(name) = source.file_name() else {
                    failures.push(format!("{} has no file name", source.display()));
                    continue;
                };
                let destination = destination_dir.join(name);
                if *source == destination {
                    continue;
                }
                if vfs.stat(&destination).await.is_ok() {
                    failures.push(format!("{} already exists", destination.display()));
                    continue;
                }
                let result = if cut {
                    vfs.rename(source, &destination).await
                } else {
                    vfs.copy(source, &destination).await
                };
                match result {
                    Ok(()) => done += 1,
                    Err(error) => failures.push(format!("{}: {error}", source.display())),
                }
            }
            let verb = if cut { "moved" } else { "copied" };
            let (level, text) = if !failures.is_empty() {
                (
                    StatusLevel::Error,
                    format!(
                        "{verb} {done} of {} entries; {}",
                        paths.len(),
                        failures.join("; ")
                    ),
                )
            } else if done == 0 {
                (StatusLevel::Info, "nothing to paste".to_string())
            } else {
                (
                    StatusLevel::Info,
                    format!(
                        "{verb} {done} {}",
                        if done == 1 { "entry" } else { "entries" }
                    ),
                )
            };
            send_notice(&tx, tab_id, load_id, level, text).await;
            if cut {
                let _ = tx
                    .send(DirUpdate::ClipboardFinished {
                        clipboard_id,
                        success: done > 0 && failures.is_empty(),
                    })
                    .await;
            }
            stream_dir(vfs, tx, destination_dir, tab_id, load_id).await;
        });
        self.op_handles.push(handle);
    }

    fn delete_selection(&mut self, mode: RemoveMode) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let action = crate::state::ConfirmAction::Delete { paths, mode };
        let title = if count == 1 {
            format!("delete {}?", Self::action_name(mode))
        } else {
            format!("delete {count} entries?")
        };
        self.state.prompt = Some(PromptState::confirm(action, title));
        self.dirty = true;
    }

    fn action_name(mode: RemoveMode) -> &'static str {
        match mode {
            RemoveMode::Trash => "to trash",
            RemoveMode::Permanent => "permanently",
        }
    }

    fn open_external(&mut self) {
        let Some(path) = self.state.selected().map(|entry| entry.path.clone()) else {
            return;
        };

        #[cfg(target_os = "linux")]
        let mut command = Command::new("xdg-open");
        #[cfg(target_os = "macos")]
        let mut command = Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = Command::new("cmd");

        #[cfg(target_os = "windows")]
        let command = command.args(["/C", "start", ""]);
        let result = command.arg(path.as_path()).spawn();
        match result {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                self.state.set_status(StatusLevel::Info, "opened");
            }
            Err(error) => self
                .state
                .set_status(StatusLevel::Error, format!("open failed: {error}")),
        }
        self.dirty = true;
    }

    fn open_palette(&mut self) {
        self.clear_pending();
        self.state.palette = Some(PaletteState::default());
        self.refresh_palette();
        self.dirty = true;
    }

    fn refresh_palette(&mut self) {
        let Some(palette) = &mut self.state.palette else {
            return;
        };

        let (query, _) = palette.command_query();
        let query = query.to_ascii_lowercase();
        palette.matches = self
            .registry
            .specs()
            .filter(|spec| {
                query.is_empty()
                    || spec.name.to_ascii_lowercase().contains(&query)
                    || spec.description.to_ascii_lowercase().contains(&query)
            })
            .map(|spec| PaletteMatch {
                name: spec.name.to_string(),
                description: spec.description.to_string(),
            })
            .collect();
        palette.selected = palette
            .selected
            .min(palette.matches.len().saturating_sub(1));
        self.dirty = true;
    }

    fn handle_palette_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.state.palette = None;
                self.dirty = true;
            }
            Key::Enter => {
                let (name, args) = self
                    .state
                    .palette
                    .as_ref()
                    .and_then(|palette| {
                        palette
                            .matches
                            .get(palette.selected)
                            .map(|entry| (entry.name.clone(), palette.command_query().1))
                    })
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.state.palette = None;
                    self.dispatch_with_args(&name, Args::new(args));
                }
            }
            Key::Backspace => {
                if let Some(palette) = &mut self.state.palette {
                    palette.query.pop();
                    palette.selected = 0;
                }
                self.refresh_palette();
            }
            Key::Up => {
                if let Some(palette) = &mut self.state.palette {
                    palette.selected = palette.selected.saturating_sub(1);
                }
                self.dirty = true;
            }
            Key::Down => {
                if let Some(palette) = &mut self.state.palette {
                    let last = palette.matches.len().saturating_sub(1);
                    palette.selected = (palette.selected + 1).min(last);
                }
                self.dirty = true;
            }
            Key::Char(ch) if !ch.is_control() => {
                if let Some(palette) = &mut self.state.palette {
                    palette.query.push(ch);
                    palette.selected = 0;
                }
                self.refresh_palette();
            }
            _ => {}
        }
    }

    fn handle_prompt_key(&mut self, key: Key) {
        if self.state.prompt.is_none() {
            return;
        }

        match key {
            Key::Esc => {
                self.state.prompt = None;
                self.dirty = true;
            }
            Key::Enter => {
                let prompt = self.state.prompt.take().expect("prompt");
                self.execute_prompt(prompt);
            }
            Key::Backspace => {
                if let Some(prompt) = self.state.prompt.as_mut() {
                    prompt.input.pop();
                }
                self.dirty = true;
            }
            Key::Char(ch) if !ch.is_control() => {
                let confirm = self
                    .state
                    .prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt.is_confirm());
                if confirm && (ch == 'y' || ch == 'Y') {
                    let prompt = self.state.prompt.take().expect("prompt");
                    self.execute_prompt(prompt);
                    return;
                }
                if confirm && (ch == 'n' || ch == 'N') {
                    self.state.prompt = None;
                    self.dirty = true;
                    return;
                }
                if let Some(prompt) = self.state.prompt.as_mut() {
                    prompt.input.push(ch);
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn execute_prompt(&mut self, prompt: PromptState) {
        match prompt.kind {
            PromptKind::Mkdir => {
                let name = match validate_name(&prompt.input) {
                    Ok(name) => name,
                    Err(error) => {
                        self.state.set_status(StatusLevel::Error, error);
                        self.dirty = true;
                        return;
                    }
                };
                let target = self.state.tab().path.join(&name);
                let (path, tab_id, load_id) = self.begin_operation();
                self.pending_select = target
                    .file_name()
                    .map(|name| (tab_id, name.to_string_lossy().into_owned()));

                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                let handle = tokio::spawn(async move {
                    if let Err(error) = vfs.create_dir(&target).await {
                        let _ = tx
                            .send(DirUpdate::Failed {
                                tab_id,
                                load_id,
                                error: error.to_string(),
                            })
                            .await;
                        return;
                    }
                    send_notice(&tx, tab_id, load_id, StatusLevel::Info, "directory created").await;
                    stream_dir(vfs, tx, path, tab_id, load_id).await;
                });
                self.op_handles.push(handle);
            }
            PromptKind::Touch => {
                let name = match validate_name(&prompt.input) {
                    Ok(name) => name,
                    Err(error) => {
                        self.state.set_status(StatusLevel::Error, error);
                        self.dirty = true;
                        return;
                    }
                };
                let target = self.state.tab().path.join(&name);
                let (path, tab_id, load_id) = self.begin_operation();
                self.pending_select = target
                    .file_name()
                    .map(|name| (tab_id, name.to_string_lossy().into_owned()));

                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                let handle = tokio::spawn(async move {
                    if let Err(error) = vfs.create_file(&target).await {
                        let _ = tx
                            .send(DirUpdate::Failed {
                                tab_id,
                                load_id,
                                error: error.to_string(),
                            })
                            .await;
                        return;
                    }
                    send_notice(&tx, tab_id, load_id, StatusLevel::Info, "file created").await;
                    stream_dir(vfs, tx, path, tab_id, load_id).await;
                });
                self.op_handles.push(handle);
            }
            PromptKind::Rename { target } => {
                let name = match validate_name(&prompt.input) {
                    Ok(name) => name,
                    Err(error) => {
                        self.state.set_status(StatusLevel::Error, error);
                        self.dirty = true;
                        return;
                    }
                };
                let destination = target
                    .parent()
                    .unwrap_or_else(|| target.clone())
                    .join(&name);
                let (path, tab_id, load_id) = self.begin_operation();
                self.pending_select = destination
                    .file_name()
                    .map(|name| (tab_id, name.to_string_lossy().into_owned()));

                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                let handle = tokio::spawn(async move {
                    if destination != target && vfs.stat(&destination).await.is_ok() {
                        let _ = tx
                            .send(DirUpdate::Failed {
                                tab_id,
                                load_id,
                                error: format!("{} already exists", destination.display()),
                            })
                            .await;
                        return;
                    }
                    if let Err(error) = vfs.rename(&target, &destination).await {
                        let _ = tx
                            .send(DirUpdate::Failed {
                                tab_id,
                                load_id,
                                error: error.to_string(),
                            })
                            .await;
                        return;
                    }
                    send_notice(&tx, tab_id, load_id, StatusLevel::Info, "renamed").await;
                    stream_dir(vfs, tx, path, tab_id, load_id).await;
                });
                self.op_handles.push(handle);
            }
            PromptKind::Filter => {
                self.state.tab_mut().filter = if prompt.input.trim().is_empty() {
                    None
                } else {
                    Some(prompt.input.trim().to_string())
                };
                self.refresh_view();
            }
            PromptKind::Search => self.start_search(prompt.input.trim().to_string()),
            PromptKind::Confirm { action } => match action {
                crate::state::ConfirmAction::Delete { paths, mode } => {
                    let (path, tab_id, load_id) = self.begin_operation();
                    self.pending_select = None;

                    let vfs = Arc::clone(&self.vfs);
                    let tx = self.dir_tx.clone();
                    let count = paths.len();
                    let handle = tokio::spawn(async move {
                        let mut failures = Vec::new();
                        for target in paths {
                            if let Err(error) = vfs.remove(&target, mode).await {
                                failures.push(format!("{}: {error}", target.display()));
                            }
                        }
                        let text = if failures.is_empty() {
                            format!(
                                "deleted {count} {}",
                                if count == 1 { "entry" } else { "entries" }
                            )
                        } else {
                            format!(
                                "deleted {} of {count} entries; {}",
                                count - failures.len(),
                                failures.join("; ")
                            )
                        };
                        let level = if failures.is_empty() {
                            StatusLevel::Info
                        } else {
                            StatusLevel::Error
                        };
                        send_notice(&tx, tab_id, load_id, level, text).await;
                        stream_dir(vfs, tx, path, tab_id, load_id).await;
                    });
                    self.op_handles.push(handle);
                }
            },
        }
        self.dirty = true;
    }

    fn dispatch(&mut self, name: &str) {
        self.dispatch_with_args(name, Args::empty());
    }

    fn dispatch_with_args(&mut self, name: &str, args: Args) {
        let effect = {
            let mut ctx = CommandContext {
                state: &mut self.state,
            };
            self.registry.dispatch(name, &mut ctx, &args)
        };

        match effect {
            Some(effect) => self.apply_effect(effect),
            None => {
                self.state
                    .set_status(StatusLevel::Error, format!("unknown command: {name}"));
                self.dirty = true;
            }
        }
    }

    fn apply_effect(&mut self, effect: Effect) {
        match effect {
            Effect::None => {}
            Effect::Redraw => self.dirty = true,
            Effect::Navigate { path, select } => self.start_load(path, select),
            Effect::CycleTheme => self.cycle_theme = true,
            Effect::ToggleIcons => self.toggle_icons = true,
            Effect::NewTab => self.open_tab(),
            Effect::CloseTab => self.close_tab(),
            Effect::NextTab => {
                let next = (self.state.active + 1) % self.state.tabs.len();
                self.switch_tab(next);
            }
            Effect::PreviousTab => {
                let previous = if self.state.active == 0 {
                    self.state.tabs.len() - 1
                } else {
                    self.state.active - 1
                };
                self.switch_tab(previous);
            }
            Effect::Quit => self.quit = true,
            Effect::Prompt(prompt) => {
                self.clear_pending();
                self.state.prompt = Some(prompt);
                self.dirty = true;
            }
            Effect::ToggleHidden => {
                self.state.settings.show_hidden = !self.state.settings.show_hidden;
                self.refresh_view();
            }
            Effect::CycleSort => {
                self.state.settings.sort.mode = self.state.settings.sort.mode.next();
                self.resort_all();
            }
            Effect::ToggleSortReverse => {
                self.state.settings.sort.reverse = !self.state.settings.sort.reverse;
                self.resort_all();
            }
            Effect::ToggleDirsFirst => {
                self.state.settings.sort.dirs_first = !self.state.settings.sort.dirs_first;
                self.resort_all();
            }
            Effect::SetFilter(filter) => {
                self.state.tab_mut().filter = filter;
                self.refresh_view();
            }
            Effect::Search(query) => self.start_search(query.unwrap_or_default()),
            Effect::ClearSearch => self.clear_search(),
            Effect::CopySelection { cut } => self.copy_selection(cut),
            Effect::PasteClipboard => self.paste_clipboard(),
            Effect::DeleteSelection { mode } => self.delete_selection(mode),
            Effect::OpenExternal => self.open_external(),
            Effect::SelectAll => {
                let settings = self.state.settings;
                self.state.tab_mut().select_all(&settings);
                self.dirty = true;
            }
            Effect::ClearMarks => {
                self.state.tab_mut().marks.clear();
                self.dirty = true;
            }
            Effect::ToggleMark => {
                let settings = self.state.settings;
                self.state.tab_mut().toggle_mark(&settings);
                self.dirty = true;
            }
        }
    }

    fn start_load(&mut self, path: VPath, select: Option<String>) {
        let tab_id = self.state.tab().id;
        self.abort_search_for_tab(tab_id);
        self.abort_directory_load();
        let load_id = {
            let tab = self.state.tab_mut();
            tab.load_id = tab.load_id.wrapping_add(1);
            tab.path = path.clone();
            tab.loading = true;
            tab.error = None;
            tab.entries.clear();
            tab.cursor = 0;
            tab.marks.clear();
            tab.filter = None;
            tab.search = None;
            tab.load_id
        };
        self.state.load_state = Some((tab_id, load_id));

        self.pending_select = select.map(|name| (tab_id, name));
        self.clear_pending();
        self.start_parent_load(&path);
        self.dirty = true;

        let vfs = Arc::clone(&self.vfs);
        let tx = self.dir_tx.clone();
        self.load_handle = Some(tokio::spawn(async move {
            stream_dir(vfs, tx, path, tab_id, load_id).await;
        }));
    }

    fn apply_dir_update(&mut self, update: DirUpdate) {
        if let DirUpdate::Notice {
            tab_id,
            load_id,
            level,
            text,
        } = &update
        {
            let current = self
                .state
                .tab_index(*tab_id)
                .is_some_and(|index| self.state.tabs[index].load_id == *load_id);
            if current {
                self.state.set_status(*level, text.clone());
                self.dirty = true;
            }
            return;
        }

        if let DirUpdate::ClipboardFinished {
            clipboard_id,
            success,
        } = update
        {
            if clipboard_id == self.clipboard_id && success {
                self.state.clipboard = None;
                self.dirty = true;
            }
            return;
        }

        let (tab_id, load_id) = match &update {
            DirUpdate::Ready {
                tab_id, load_id, ..
            }
            | DirUpdate::Entry {
                tab_id, load_id, ..
            }
            | DirUpdate::Finished { tab_id, load_id }
            | DirUpdate::Failed {
                tab_id, load_id, ..
            } => (*tab_id, *load_id),
            DirUpdate::Notice { .. } | DirUpdate::ClipboardFinished { .. } => return,
        };
        let Some(index) = self.state.tab_index(tab_id) else {
            return;
        };
        if self.state.tabs[index].load_id != load_id {
            return;
        }

        match update {
            DirUpdate::Ready { path, .. } => {
                let tab = &mut self.state.tabs[index];
                tab.path = path;
                tab.entries.clear();
                tab.cursor = 0;
                tab.loading = true;
                tab.error = None;
                if index == self.state.active {
                    self.invalidate_preview();
                }
                self.dirty = true;
            }
            DirUpdate::Entry { entry, .. } => {
                self.state.tabs[index].entries.push(Entry::from(entry));
                self.dirty = true;
            }
            DirUpdate::Finished { .. } => {
                self.load_retries = 0;
                let settings = self.state.settings;
                let tab = &mut self.state.tabs[index];
                let hovered = if tab.cursor > 0 {
                    tab.selected(&settings).map(|entry| entry.path.clone())
                } else {
                    None
                };

                sort_entries(&mut tab.entries, settings.sort);
                tab.loading = false;

                if self
                    .pending_select
                    .as_ref()
                    .is_some_and(|(id, _)| *id == tab_id)
                {
                    let name = self.pending_select.take().expect("pending selection").1;
                    tab.cursor = tab.cursor_for_name(&settings, &name).unwrap_or(0);
                } else if let Some(path) = hovered
                    && let Some(cursor) = tab.cursor_for_path(&settings, &path)
                {
                    tab.cursor = cursor;
                }

                tab.clamp_cursor(&settings);
                self.dirty = true;
            }
            DirUpdate::Failed { error, .. } => {
                self.pending_select = None;
                if self.load_retries >= 2 {
                    let tab = &mut self.state.tabs[index];
                    tab.error = Some(error.clone());
                    tab.loading = false;
                    self.load_handle = None;
                    self.state.load_state = None;
                    if index == self.state.active {
                        self.state.set_status(StatusLevel::Error, error);
                    }
                    self.dirty = true;
                    return;
                }
                self.load_retries += 1;
                let retry = {
                    let tab = &mut self.state.tabs[index];
                    tab.error = None;
                    tab.load_id = tab.load_id.wrapping_add(1);
                    tab.loading = true;
                    (tab.path.clone(), tab.load_id)
                };
                if index == self.state.active {
                    self.state.set_status(StatusLevel::Error, error);
                }
                if let Some(handle) = self.load_handle.take() {
                    handle.abort();
                }
                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                let (path, load_id) = retry;
                self.state.load_state = Some((tab_id, load_id));
                self.load_handle = Some(tokio::spawn(async move {
                    stream_dir(vfs, tx, path, tab_id, load_id).await;
                }));
                self.dirty = true;
            }
            DirUpdate::Notice { .. } | DirUpdate::ClipboardFinished { .. } => {}
        }
    }

    fn start_search(&mut self, query: String) {
        if query.is_empty() {
            self.state
                .set_status(StatusLevel::Info, "enter a search query");
            self.dirty = true;
            return;
        }

        let tab_id = self.state.tab().id;
        self.search_id = self.search_id.wrapping_add(1);
        let search_id = self.search_id;
        if let Some(owner) = self.search_owner.take() {
            if let Some(handle) = self.search_handle.take() {
                handle.abort();
            }
            if let Some(index) = self.state.tab_index(owner)
                && let Some(search) = self.state.tabs[index].search.as_mut()
            {
                search.id = search_id;
                search.loading = false;
            }
        }
        self.abort_directory_load();

        let (root, previous_entries, previous_cursor, previous_filter) = {
            let tab = self.state.tab();
            match &tab.search {
                Some(search) => (
                    search.root.clone(),
                    search.previous_entries.clone(),
                    search.previous_cursor,
                    search.previous_filter.clone(),
                ),
                None => (
                    tab.path.clone(),
                    tab.entries.clone(),
                    tab.cursor,
                    tab.filter.clone(),
                ),
            }
        };
        {
            let tab = self.state.tab_mut();
            tab.load_id = tab.load_id.wrapping_add(1);
            tab.loading = false;
            tab.error = None;
            tab.entries.clear();
            tab.cursor = 0;
            tab.filter = None;
            tab.search = Some(SearchState {
                id: search_id,
                query: query.clone(),
                root: root.clone(),
                previous_entries,
                previous_cursor,
                previous_filter,
                loading: true,
                error: None,
                matches: 0,
            });
        }
        self.search_owner = Some(tab_id);
        self.state
            .set_status(StatusLevel::Info, format!("searching for {query}..."));
        self.dirty = true;

        let vfs = Arc::clone(&self.vfs);
        let tx = self.search_tx.clone();
        self.search_handle = Some(tokio::spawn(async move {
            walk_search(vfs, tx, root, query, tab_id, search_id).await;
        }));
    }

    fn clear_search(&mut self) {
        let tab_id = self.state.tab().id;
        let Some(index) = self.state.tab_index(tab_id) else {
            return;
        };
        let Some(search) = self.state.tabs[index].search.take() else {
            return;
        };
        if self.search_owner == Some(tab_id) {
            if let Some(handle) = self.search_handle.take() {
                handle.abort();
            }
            self.search_owner = None;
        }
        let tab = &mut self.state.tabs[index];
        tab.entries = search.previous_entries;
        tab.cursor = search.previous_cursor;
        tab.filter = search.previous_filter;
        tab.loading = false;
        tab.error = None;
        self.state.set_status(StatusLevel::Info, "search cleared");
        self.refresh_view();
    }

    fn abort_search_for_tab(&mut self, tab_id: u64) {
        if self.search_owner != Some(tab_id) {
            return;
        }
        if let Some(handle) = self.search_handle.take() {
            handle.abort();
        }
        self.search_owner = None;
        self.search_id = self.search_id.wrapping_add(1);
        let invalid_id = self.search_id;
        if let Some(index) = self.state.tab_index(tab_id)
            && let Some(search) = self.state.tabs[index].search.as_mut()
        {
            search.id = invalid_id;
            search.loading = false;
        }
    }

    fn abort_directory_load(&mut self) {
        self.load_retries = 0;
        if let Some((owner, _)) = self.state.load_state.take()
            && let Some(index) = self.state.tab_index(owner)
        {
            self.state.tabs[index].loading = false;
        }
        if let Some(handle) = self.load_handle.take() {
            handle.abort();
        }
    }

    fn apply_search_update(&mut self, update: SearchUpdate) {
        let (tab_id, search_id) = match &update {
            SearchUpdate::Entry {
                tab_id, search_id, ..
            }
            | SearchUpdate::Finished { tab_id, search_id }
            | SearchUpdate::Failed {
                tab_id, search_id, ..
            }
            | SearchUpdate::Notice {
                tab_id, search_id, ..
            } => (*tab_id, *search_id),
        };
        let Some(index) = self.state.tab_index(tab_id) else {
            return;
        };
        if !self.state.tabs[index]
            .search
            .as_ref()
            .is_some_and(|search| search.id == search_id)
        {
            return;
        }

        match update {
            SearchUpdate::Entry { entry, .. } => {
                self.state.tabs[index].entries.push(entry);
                if let Some(search) = self.state.tabs[index].search.as_mut() {
                    search.matches += 1;
                }
                self.dirty = true;
            }
            SearchUpdate::Finished { .. } => {
                let query = self.state.tabs[index]
                    .search
                    .as_ref()
                    .map(|search| search.query.clone())
                    .unwrap_or_default();
                if let Some(search) = self.state.tabs[index].search.as_mut() {
                    search.loading = false;
                }
                let settings = self.state.settings;
                {
                    let tab = &mut self.state.tabs[index];
                    sort_entries(&mut tab.entries, settings.sort);
                    tab.clamp_cursor(&settings);
                }
                if self.search_owner == Some(tab_id) {
                    self.search_handle = None;
                    self.search_owner = None;
                }
                if index == self.state.active {
                    self.refresh_view();
                    self.state.set_status(
                        StatusLevel::Info,
                        format!(
                            "{} results for {query}",
                            self.state.tabs[index].entries.len()
                        ),
                    );
                }
                self.dirty = true;
            }
            SearchUpdate::Failed { error, .. } => {
                if let Some(search) = self.state.tabs[index].search.as_mut() {
                    search.loading = false;
                    search.error = Some(error.clone());
                }
                if self.search_owner == Some(tab_id) {
                    if let Some(handle) = self.search_handle.take() {
                        handle.abort();
                    }
                    self.search_owner = None;
                }
                if index == self.state.active {
                    self.state.set_status(StatusLevel::Error, error);
                }
                self.dirty = true;
            }
            SearchUpdate::Notice { text, .. } => {
                if index == self.state.active {
                    self.state.set_status(StatusLevel::Info, text);
                }
                self.dirty = true;
            }
        }
    }

    fn sync_preview(&mut self) {
        if self.state.tab().loading {
            return;
        }

        let target = self.state.selected().map(|entry| entry.path.clone());
        if target == self.state.preview.path {
            return;
        }

        let Some(entry) = self.state.selected().cloned() else {
            self.invalidate_preview();
            return;
        };

        self.start_preview(entry);
    }

    fn open_tab(&mut self) {
        self.state.open_tab();
        let path = self.state.tab().path.clone();
        self.clear_pending();
        self.invalidate_preview();
        self.start_load(path, None);
        self.dirty = true;
    }

    fn close_tab(&mut self) {
        let closed_id = self.state.tab().id;
        if !self.state.close_active_tab() {
            self.state
                .set_status(StatusLevel::Info, "cannot close the last tab");
            self.dirty = true;
            return;
        }

        self.pending_select = None;
        self.abort_search_for_tab(closed_id);
        self.clear_pending();
        self.invalidate_preview();
        let needs_load = {
            let tab = self.state.tab();
            tab.loading || tab.entries.is_empty()
        };
        if needs_load {
            let path = self.state.tab().path.clone();
            self.start_load(path, None);
        } else {
            let path = self.state.tab().path.clone();
            self.start_parent_load(&path);
        }
        self.dirty = true;
    }

    fn switch_tab(&mut self, index: usize) {
        if index == self.state.active || index >= self.state.tabs.len() {
            return;
        }

        self.abort_directory_load();
        self.pending_select = None;
        self.state.active = index;
        self.clear_pending();
        self.invalidate_preview();

        if self.state.tab().loading {
            let path = self.state.tab().path.clone();
            self.start_load(path, None);
        } else {
            let path = self.state.tab().path.clone();
            self.start_parent_load(&path);
        }
        self.dirty = true;
    }

    fn start_parent_load(&mut self, path: &VPath) {
        self.parent_id = self.parent_id.wrapping_add(1);
        let id = self.parent_id;

        if let Some(handle) = self.parent_handle.take() {
            handle.abort();
        }

        let Some(parent_path) = path.parent() else {
            self.state.parent = crate::state::ParentState::default();
            return;
        };

        self.state.parent = crate::state::ParentState {
            path: Some(parent_path.clone()),
            entries: Vec::new(),
            loading: true,
            error: None,
        };

        let vfs = Arc::clone(&self.vfs);
        let tx = self.parent_tx.clone();
        let settings = self.state.settings;
        self.parent_handle = Some(tokio::spawn(async move {
            let (entries, error) = match load_directory_preview(vfs, &parent_path, settings).await {
                Preview::Directory { entries, .. } => (entries, None),
                Preview::Error(error) => (Vec::new(), Some(error)),
                _ => (Vec::new(), None),
            };
            let _ = tx.send(ParentUpdate { id, entries, error }).await;
        }));
    }

    fn apply_parent_update(&mut self, update: ParentUpdate) {
        if update.id != self.parent_id {
            return;
        }
        self.state.parent.entries = update.entries;
        self.state.parent.error = update.error;
        self.state.parent.loading = false;
        self.dirty = true;
    }

    fn start_preview(&mut self, entry: Entry) {
        self.preview_id = self.preview_id.wrapping_add(1);
        let id = self.preview_id;

        if let Some(handle) = self.preview_handle.take() {
            handle.abort();
        }

        self.state.preview = PreviewState {
            path: Some(entry.path.clone()),
            content: Preview::Loading,
            scroll: 0,
            focus: self.state.preview.focus,
        };
        self.dirty = true;

        let vfs = Arc::clone(&self.vfs);
        let tx = self.preview_tx.clone();
        let settings = self.state.settings;
        self.preview_handle = Some(tokio::spawn(async move {
            let content = load_preview(vfs, entry, settings).await;
            let _ = tx.send(PreviewUpdate { id, content }).await;
        }));
    }

    fn invalidate_preview(&mut self) {
        self.preview_id = self.preview_id.wrapping_add(1);
        if let Some(handle) = self.preview_handle.take() {
            handle.abort();
        }
        self.state.preview = PreviewState::default();
        self.dirty = true;
    }

    fn apply_preview_update(&mut self, update: PreviewUpdate) {
        if update.id != self.preview_id {
            return;
        }
        self.state.preview.content = update.content;
        self.state.preview.scroll = 0;
        self.dirty = true;
    }
}

async fn send_notice(
    tx: &mpsc::Sender<DirUpdate>,
    tab_id: u64,
    load_id: u64,
    level: StatusLevel,
    text: impl Into<String>,
) {
    let _ = tx
        .send(DirUpdate::Notice {
            tab_id,
            load_id,
            level,
            text: text.into(),
        })
        .await;
}

async fn stream_dir(
    vfs: Arc<dyn Vfs>,
    tx: mpsc::Sender<DirUpdate>,
    path: VPath,
    tab_id: u64,
    load_id: u64,
) {
    match vfs.read_dir(&path).await {
        Ok(mut stream) => {
            if tx
                .send(DirUpdate::Ready {
                    tab_id,
                    load_id,
                    path,
                })
                .await
                .is_err()
            {
                return;
            }
            while let Some(item) = stream.next().await {
                match item {
                    Ok(entry) => {
                        if tx
                            .send(DirUpdate::Entry {
                                tab_id,
                                load_id,
                                entry,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => continue,
                }
            }
            let _ = tx.send(DirUpdate::Finished { tab_id, load_id }).await;
        }
        Err(error) => {
            let _ = tx
                .send(DirUpdate::Failed {
                    tab_id,
                    load_id,
                    error: error.to_string(),
                })
                .await;
        }
    }
}

async fn walk_search(
    vfs: Arc<dyn Vfs>,
    tx: mpsc::Sender<SearchUpdate>,
    root: VPath,
    query: String,
    tab_id: u64,
    search_id: u64,
) {
    let needle = query.to_lowercase();
    let root_dir = root.clone();
    let mut stack = vec![root];
    let mut visited = HashSet::new();

    while let Some(dir) = stack.pop() {
        if !visited.insert(dir.clone()) {
            continue;
        }
        let mut stream = match vfs.read_dir(&dir).await {
            Ok(stream) => stream,
            Err(error) => {
                if dir == root_dir {
                    let _ = tx
                        .send(SearchUpdate::Failed {
                            tab_id,
                            search_id,
                            error: error.to_string(),
                        })
                        .await;
                    return;
                }
                let _ = tx
                    .send(SearchUpdate::Notice {
                        tab_id,
                        search_id,
                        text: format!("skipped {}: {error}", dir.display()),
                    })
                    .await;
                continue;
            }
        };

        while let Some(item) = stream.next().await {
            match item {
                Ok(raw) => {
                    let entry = Entry::from(raw);
                    if entry.name.to_lowercase().contains(&needle)
                        && tx
                            .send(SearchUpdate::Entry {
                                tab_id,
                                search_id,
                                entry: entry.clone(),
                            })
                            .await
                            .is_err()
                    {
                        return;
                    }
                    if entry.kind == EntryKind::Dir && !entry.is_symlink {
                        stack.push(entry.path.clone());
                    }
                }
                Err(error) => {
                    let _ = tx
                        .send(SearchUpdate::Notice {
                            tab_id,
                            search_id,
                            text: format!("skipped entry: {error}"),
                        })
                        .await;
                }
            }
        }
    }

    let _ = tx.send(SearchUpdate::Finished { tab_id, search_id }).await;
}

fn validate_name(name: &str) -> std::result::Result<String, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a name is required");
    }
    if name == "." || name == ".." || name.chars().any(|ch| ch.is_control() || is_separator(ch)) {
        return Err("name must be a single file name");
    }
    Ok(name.to_string())
}

fn is_separator(ch: char) -> bool {
    matches!(ch, '/' | '\\')
}

async fn load_preview(vfs: Arc<dyn Vfs>, entry: Entry, settings: Settings) -> Preview {
    match entry.kind {
        hwe_vfs::EntryKind::Dir => load_directory_preview(vfs, &entry.path, settings).await,
        hwe_vfs::EntryKind::File => load_text_preview(vfs, &entry).await,
        hwe_vfs::EntryKind::Other => Preview::Other,
    }
}

async fn load_directory_preview(vfs: Arc<dyn Vfs>, path: &VPath, settings: Settings) -> Preview {
    let mut stream = match vfs.read_dir(path).await {
        Ok(stream) => stream,
        Err(error) => return Preview::Error(error.to_string()),
    };

    let mut entries = Vec::new();
    while entries.len() < MAX_PREVIEW_ENTRIES {
        match stream.next().await {
            Some(Ok(item)) => entries.push(Entry::from(item)),
            Some(Err(_)) => continue,
            None => break,
        }
    }

    let truncated = entries.len() == MAX_PREVIEW_ENTRIES && stream.next().await.is_some();
    entries.retain(|entry| settings.show_hidden || !entry.is_hidden);
    sort_entries(&mut entries, settings.sort);
    Preview::Directory { entries, truncated }
}

async fn load_text_preview(vfs: Arc<dyn Vfs>, entry: &Entry) -> Preview {
    let reader = match vfs.open_read(&entry.path).await {
        Ok(reader) => reader,
        Err(error) => return Preview::Error(error.to_string()),
    };

    let mut reader = reader.take(MAX_PREVIEW_BYTES + 1);
    let mut bytes = Vec::new();
    if let Err(error) = reader.read_to_end(&mut bytes).await {
        return Preview::Error(error.to_string());
    }

    let truncated_by_bytes = bytes.len() as u64 > MAX_PREVIEW_BYTES;
    if truncated_by_bytes {
        bytes.truncate(MAX_PREVIEW_BYTES as usize);
    }

    if looks_binary(&bytes) {
        return Preview::Binary { size: entry.size };
    }

    let text = String::from_utf8_lossy(&bytes);
    let all_lines: Vec<&str> = text.lines().collect();
    let total_lines = all_lines.len();
    let truncated_by_lines = total_lines > MAX_PREVIEW_LINES;
    let lines = all_lines
        .iter()
        .take(MAX_PREVIEW_LINES)
        .map(|line| sanitize_line(line))
        .collect();
    let truncated = truncated_by_bytes || truncated_by_lines;

    Preview::Text { lines, truncated }
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return true;
    }
    let controls = bytes
        .iter()
        .filter(|&&byte| byte.is_ascii_control() && !matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c))
        .count();
    controls.saturating_mul(20) > bytes.len()
}

fn sanitize_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        match ch {
            '\t' => out.push_str("    "),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use hwe_vfs::{EntryKind, LocalFs, VPath};

    use super::{
        App, Renderer, SearchUpdate, load_directory_preview, load_preview, looks_binary, mpsc,
        validate_name,
    };
    use crate::event::AppEvent;
    use crate::keymap::Key;
    use crate::state::{AppState, Entry, Preview, Settings};

    struct RecordingRenderer {
        events: mpsc::UnboundedSender<AppEvent>,
        frames: Arc<Mutex<Vec<Vec<String>>>>,
        quit_sent: bool,
    }

    impl Renderer for RecordingRenderer {
        fn draw(&mut self, state: &AppState) -> io::Result<()> {
            if !state.tab().loading {
                self.frames.lock().unwrap().push(
                    state
                        .visible_entries()
                        .iter()
                        .map(|entry| entry.name.clone())
                        .collect(),
                );
                if !self.quit_sent {
                    self.quit_sent = true;
                    let _ = self.events.send(AppEvent::Key(Key::Char('q')));
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn loads_sorts_and_quits() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"b").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let (events, event_rx) = mpsc::unbounded_channel();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let mut renderer = RecordingRenderer {
            events,
            frames: Arc::clone(&frames),
            quit_sent: false,
        };

        let app = App::new(
            Arc::new(LocalFs::new()),
            VPath::new(dir.path()),
            event_rx,
            Settings::default(),
        );
        tokio::time::timeout(Duration::from_secs(5), app.run(&mut renderer))
            .await
            .expect("app should quit")
            .expect("event loop");

        let frames = frames.lock().unwrap();
        let names = frames.last().expect("a rendered frame");
        assert_eq!(
            names,
            &["sub".to_string(), "a.txt".to_string(), "b.txt".to_string()]
        );
    }

    async fn rendered_names(path: &std::path::Path, settings: Settings) -> Vec<String> {
        let (events, event_rx) = mpsc::unbounded_channel();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let mut renderer = RecordingRenderer {
            events,
            frames: Arc::clone(&frames),
            quit_sent: false,
        };
        let app = App::new(
            Arc::new(LocalFs::new()),
            VPath::new(path),
            event_rx,
            settings,
        );
        tokio::time::timeout(Duration::from_secs(5), app.run(&mut renderer))
            .await
            .expect("app should quit")
            .expect("event loop");
        frames.lock().unwrap().last().cloned().unwrap_or_default()
    }

    #[tokio::test]
    async fn hidden_files_follow_the_setting() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join(".secret"), b"s").unwrap();

        let names = rendered_names(dir.path(), Settings::default()).await;
        assert_eq!(names, ["a.txt"]);

        let settings = Settings {
            show_hidden: true,
            ..Settings::default()
        };
        let names = rendered_names(dir.path(), settings).await;
        assert_eq!(names, [".secret", "a.txt"]);
    }

    #[test]
    fn prompt_names_cannot_escape_the_directory() {
        assert_eq!(validate_name("  note.txt  "), Ok("note.txt".to_string()));
        for invalid in ["", "   ", "..", "../escape", "/tmp/escape", "a/../../b"] {
            assert!(validate_name(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    fn preview_entry(path: &std::path::Path) -> Entry {
        let meta = std::fs::metadata(path).unwrap();
        Entry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path: VPath::new(path),
            kind: if meta.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            size: Some(meta.len()),
            modified: meta.modified().ok(),
            is_hidden: false,
            is_symlink: false,
            is_executable: false,
            mode: None,
        }
    }

    #[tokio::test]
    async fn chained_searches_restore_the_original_listing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"a").unwrap();
        std::fs::write(dir.path().join("stale.txt"), b"stale").unwrap();
        let original = preview_entry(&path);
        let stale = preview_entry(&dir.path().join("stale.txt"));
        let (_, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            Arc::new(LocalFs::new()),
            VPath::new(dir.path()),
            event_rx,
            Settings::default(),
        );
        app.state.tab_mut().entries = vec![original.clone()];
        app.state.tab_mut().loading = false;

        app.start_search("a".into());
        let old_id = app.state.tab().search.as_ref().expect("search").id;
        app.start_search("stale".into());
        assert_ne!(old_id, app.state.tab().search.as_ref().expect("search").id);

        app.apply_search_update(SearchUpdate::Entry {
            tab_id: app.state.tab().id,
            search_id: old_id,
            entry: stale,
        });
        assert!(app.state.tab().entries.is_empty());

        app.clear_search();
        assert_eq!(app.state.tab().entries.len(), 1);
        assert_eq!(app.state.tab().entries[0].name, "a.txt");
        assert!(app.state.tab().search.is_none());
    }

    #[tokio::test]
    async fn starting_search_during_a_load_does_not_stay_loading() {
        let (_, event_rx) = mpsc::unbounded_channel();
        let mut app = App::new(
            Arc::new(LocalFs::new()),
            VPath::new("/tmp"),
            event_rx,
            Settings::default(),
        );
        let tab_id = app.state.tab().id;
        app.state.tab_mut().loading = true;
        app.state.load_state = Some((tab_id, 7));

        app.start_search("note".into());

        assert!(!app.state.tab().loading);
        assert!(app.state.load_state.is_none());
        assert!(app.state.tab().search.as_ref().expect("search").loading);
    }

    #[tokio::test]
    async fn previews_text_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("note.txt");
        std::fs::write(&path, b"hello\nworld\n").unwrap();

        let preview = load_preview(
            Arc::new(LocalFs::new()),
            preview_entry(&path),
            Settings::default(),
        )
        .await;
        match preview {
            Preview::Text { lines, truncated } => {
                assert_eq!(lines, vec!["hello".to_string(), "world".to_string()]);
                assert!(!truncated);
            }
            other => panic!("unexpected preview: {other:?}"),
        }
    }

    #[tokio::test]
    async fn previews_binary_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("blob.bin");
        std::fs::write(&path, b"abc\0def").unwrap();

        let preview = load_preview(
            Arc::new(LocalFs::new()),
            preview_entry(&path),
            Settings::default(),
        )
        .await;
        match preview {
            Preview::Binary { size } => assert_eq!(size, Some(7)),
            other => panic!("unexpected preview: {other:?}"),
        }
    }

    #[tokio::test]
    async fn previews_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("inner.txt"), b"x").unwrap();
        std::fs::write(sub.join("aaa.txt"), b"y").unwrap();

        let preview = load_preview(
            Arc::new(LocalFs::new()),
            preview_entry(&sub),
            Settings::default(),
        )
        .await;
        match preview {
            Preview::Directory { entries, truncated } => {
                let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
                assert_eq!(names, ["aaa.txt", "inner.txt"]);
                assert!(!truncated);
            }
            other => panic!("unexpected preview: {other:?}"),
        }
    }

    #[test]
    fn prompt_names_must_be_single_components() {
        assert_eq!(validate_name("note.txt"), Ok("note.txt".to_string()));
        for invalid in ["sub/note.txt", "sub\\note.txt", "a\nb", ".", ".."] {
            assert!(validate_name(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn binary_preview_detects_non_text_bytes() {
        assert!(looks_binary(&[0x41, 0x00, 0x42]));
        assert!(looks_binary(&[0x01, 0x02, 0x03, 0x04, 0x41]));
        assert!(!looks_binary(b"hello\nworld\t!\n"));
    }

    #[tokio::test]
    async fn directory_preview_honors_hidden_setting() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("visible.txt"), b"x").unwrap();
        std::fs::write(dir.path().join(".hidden"), b"y").unwrap();

        let preview = load_directory_preview(
            Arc::new(LocalFs::new()),
            &VPath::new(dir.path()),
            Settings::default(),
        )
        .await;
        let entries = match preview {
            Preview::Directory { entries, .. } => entries,
            other => panic!("unexpected preview: {other:?}"),
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "visible.txt");

        let settings = Settings {
            show_hidden: true,
            ..Settings::default()
        };
        let preview =
            load_directory_preview(Arc::new(LocalFs::new()), &VPath::new(dir.path()), settings)
                .await;
        let entries = match preview {
            Preview::Directory { entries, .. } => entries,
            other => panic!("unexpected preview: {other:?}"),
        };
        assert_eq!(entries.len(), 2);
    }

    struct ScriptedRenderer {
        events: mpsc::UnboundedSender<AppEvent>,
        frames: Arc<Mutex<Vec<Vec<String>>>>,
        started: bool,
        frames_after_start: usize,
        quit_sent: bool,
    }

    impl Renderer for ScriptedRenderer {
        fn draw(&mut self, state: &AppState) -> io::Result<()> {
            let names: Vec<String> = state
                .visible_entries()
                .iter()
                .map(|entry| entry.name.clone())
                .collect();
            self.frames.lock().unwrap().push(names.clone());

            if !self.started && !state.tab().loading && !names.is_empty() {
                self.started = true;
                let _ = self.events.send(AppEvent::Key(Key::Char('r')));
                for _ in 0..5 {
                    let _ = self.events.send(AppEvent::Key(Key::Backspace));
                }
                for ch in "b.txt".chars() {
                    let _ = self.events.send(AppEvent::Key(Key::Char(ch)));
                }
                let _ = self.events.send(AppEvent::Key(Key::Enter));
            } else if self.started && !state.tab().loading {
                self.frames_after_start += 1;
                if !self.quit_sent && self.frames_after_start >= 2 {
                    self.quit_sent = true;
                    let _ = self.events.send(AppEvent::Key(Key::Char('q')));
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn failed_rename_restores_the_directory_listing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"b").unwrap();

        let (events, event_rx) = mpsc::unbounded_channel();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let mut renderer = ScriptedRenderer {
            events,
            frames: Arc::clone(&frames),
            started: false,
            frames_after_start: 0,
            quit_sent: false,
        };
        let app = App::new(
            Arc::new(LocalFs::new()),
            VPath::new(dir.path()),
            event_rx,
            Settings::default(),
        );
        tokio::time::timeout(Duration::from_secs(5), app.run(&mut renderer))
            .await
            .expect("app should quit")
            .expect("event loop");

        let frames = frames.lock().unwrap();
        assert!(
            frames
                .iter()
                .any(|names| names == &["a.txt".to_string(), "b.txt".to_string()]),
            "directory listing should be restored after a failed rename: {frames:?}"
        );
    }

    struct SearchRenderer {
        events: mpsc::UnboundedSender<AppEvent>,
        frames: Arc<Mutex<Vec<Vec<String>>>>,
        started: bool,
        quit_sent: bool,
    }

    impl Renderer for SearchRenderer {
        fn draw(&mut self, state: &AppState) -> io::Result<()> {
            let names: Vec<String> = state
                .visible_entries()
                .iter()
                .map(|entry| entry.name.clone())
                .collect();
            self.frames.lock().unwrap().push(names.clone());

            if !self.started && !state.tab().loading && !names.is_empty() {
                self.started = true;
                let _ = self.events.send(AppEvent::Key(Key::ctrl('/')));
                for ch in "txt".chars() {
                    let _ = self.events.send(AppEvent::Key(Key::Char(ch)));
                }
                let _ = self.events.send(AppEvent::Key(Key::Enter));
            } else if self.started
                && !self.quit_sent
                && state
                    .tab()
                    .search
                    .as_ref()
                    .is_some_and(|search| !search.loading)
                && names.iter().any(|name| name == "nested.txt")
            {
                self.quit_sent = true;
                let _ = self.events.send(AppEvent::Key(Key::Esc));
                let _ = self.events.send(AppEvent::Key(Key::Char('q')));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn recursive_search_finds_nested_matches_and_clears() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("note.txt"), b"note").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/nested.txt"), b"nested").unwrap();

        let (events, event_rx) = mpsc::unbounded_channel();
        let frames = Arc::new(Mutex::new(Vec::new()));
        let mut renderer = SearchRenderer {
            events,
            frames: Arc::clone(&frames),
            started: false,
            quit_sent: false,
        };
        let app = App::new(
            Arc::new(LocalFs::new()),
            VPath::new(dir.path()),
            event_rx,
            Settings::default(),
        );
        tokio::time::timeout(Duration::from_secs(5), app.run(&mut renderer))
            .await
            .expect("app should quit")
            .expect("event loop");

        let frames = frames.lock().unwrap();
        assert!(
            frames
                .iter()
                .any(|names| names.iter().any(|name| name == "nested.txt")),
            "recursive search should include nested matches: {frames:?}"
        );
        assert!(
            frames
                .iter()
                .any(|names| names.as_slice() == ["sub", "note.txt"]),
            "clearing search should restore the original listing: {frames:?}"
        );
    }
}
