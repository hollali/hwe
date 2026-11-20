use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use hwe_vfs::{DirEntry as VfsDirEntry, RemoveMode, VPath, Vfs};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::state::{
    AppState, Clipboard, Entry, PaletteMatch, PaletteState, Preview, PreviewState, PromptKind,
    PromptState, Settings, SortSettings, StatusLevel, sort_entries,
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

pub struct App {
    state: AppState,
    registry: CommandRegistry,
    keymap: Keymap,
    vfs: Arc<dyn Vfs>,
    events: mpsc::UnboundedReceiver<AppEvent>,
    dir_tx: mpsc::Sender<DirUpdate>,
    dir_rx: mpsc::Receiver<DirUpdate>,
    load_handle: Option<JoinHandle<()>>,
    preview_tx: mpsc::Sender<PreviewUpdate>,
    preview_rx: mpsc::Receiver<PreviewUpdate>,
    preview_handle: Option<JoinHandle<()>>,
    preview_id: u64,
    parent_tx: mpsc::Sender<ParentUpdate>,
    parent_rx: mpsc::Receiver<ParentUpdate>,
    parent_handle: Option<JoinHandle<()>>,
    parent_id: u64,
    pending_select: Option<String>,
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
            preview_tx,
            preview_rx,
            preview_handle: None,
            preview_id: 0,
            parent_tx,
            parent_rx,
            parent_handle: None,
            parent_id: 0,
            pending_select: None,
            cycle_theme: false,
            toggle_icons: false,
            dirty: true,
            quit: false,
        }
    }

    pub async fn run(mut self, renderer: &mut dyn Renderer) -> std::io::Result<()> {
        let start = self.state.tab().path.clone();
        self.start_load(start, None);

        let mut ticker = tokio::time::interval(FRAME_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if self.state.expire_status() || self.state.tab().loading {
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

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize { .. } => self.dirty = true,
            AppEvent::Paste(text) => {
                if let Some(prompt) = &mut self.state.prompt {
                    prompt.input.push_str(&text);
                    self.dirty = true;
                }
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

        if let Some(prompt) = self.state.prompt.clone() {
            self.handle_prompt_key(key, prompt);
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
                    self.state.pending_keys.clear();
                    self.dirty = true;
                }
            }
            Resolution::Command(name) => {
                self.state.pending_keys.clear();
                self.dispatch(name);
            }
        }
    }

    fn handle_preview_key(&mut self, key: Key) -> bool {
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
        self.state.preview.scroll = next.min(length);
        self.dirty = true;
    }

    fn selected_paths(&self) -> Vec<VPath> {
        let tab = self.state.tab();
        if tab.marks.is_empty() {
            tab.selected()
                .map(|entry| entry.path.clone())
                .into_iter()
                .collect()
        } else {
            tab.marks
                .iter()
                .filter_map(|index| tab.entries.get(*index).map(|entry| entry.path.clone()))
                .collect()
        }
    }

    fn resort_current(&mut self) {
        let sort = self.state.settings.sort;
        let tab = self.state.tab_mut();
        sort_entries(&mut tab.entries, sort);
        tab.clamp_cursor();
        self.dirty = true;
    }

    fn copy_selection(&mut self, cut: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.state.clipboard = Some(Clipboard { paths, cut });
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
        let destination_dir = self.state.tab().path.clone();
        let tab_id = self.state.tab().id;
        let load_id = {
            let tab = self.state.tab_mut();
            tab.load_id = tab.load_id.wrapping_add(1);
            tab.loading = true;
            tab.error = None;
            tab.entries.clear();
            tab.cursor = 0;
            tab.load_id
        };
        self.state.load_state = Some((tab_id, load_id));
        if let Some(handle) = self.load_handle.take() {
            handle.abort();
        }
        let cut = clipboard.cut;
        self.start_parent_load(&destination_dir);
        self.dirty = true;
        if cut {
            self.state.clipboard = None;
        }

        let vfs = Arc::clone(&self.vfs);
        let paths = clipboard.paths.clone();
        let tx = self.dir_tx.clone();
        tokio::spawn(async move {
            for source in &paths {
                let Some(name) = source.file_name() else {
                    continue;
                };
                let destination = destination_dir.join(name);
                if cut {
                    let _ = vfs.rename(source, &destination).await;
                } else {
                    let _ = vfs.copy(source, &destination).await;
                }
            }
            match vfs.read_dir(&destination_dir).await {
                Ok(mut stream) => {
                    if tx
                        .send(DirUpdate::Ready {
                            tab_id,
                            load_id,
                            path: destination_dir.clone(),
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
                            Err(error) => {
                                let _ = tx
                                    .send(DirUpdate::Failed {
                                        tab_id,
                                        load_id,
                                        error: error.to_string(),
                                    })
                                    .await;
                                return;
                            }
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
        });
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
        let Some(entry) = self.state.tab().selected().cloned() else {
            return;
        };
        let path = entry.path.clone();
        tokio::spawn(async move {
            #[cfg(target_os = "linux")]
            let _ = Command::new("xdg-open").arg(path.as_path()).spawn();
            #[cfg(target_os = "macos")]
            let _ = Command::new("open").arg(path.as_path()).spawn();
            #[cfg(target_os = "windows")]
            let _ = Command::new("cmd")
                .args(["/C", "start", ""])
                .arg(path.as_path())
                .spawn();
        });
        self.state.set_status(StatusLevel::Info, "opened");
        self.dirty = true;
    }

    fn open_palette(&mut self) {
        self.state.pending_keys.clear();
        self.state.palette = Some(PaletteState::default());
        self.refresh_palette();
        self.dirty = true;
    }

    fn refresh_palette(&mut self) {
        let Some(palette) = &mut self.state.palette else {
            return;
        };

        let query = palette.query.to_ascii_lowercase();
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
                let name = self
                    .state
                    .palette
                    .as_ref()
                    .and_then(|palette| palette.matches.get(palette.selected))
                    .map(|entry| entry.name.clone());
                if let Some(name) = name {
                    self.state.palette = None;
                    self.dispatch(&name);
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

    fn handle_prompt_key(&mut self, key: Key, _prompt: PromptState) {
        if self.state.prompt.is_none() {
            return;
        }

        match key {
            Key::Esc => {
                self.state.prompt = None;
                self.dirty = true;
            }
            Key::Enter => {
                let prompt = self.state.prompt.take().unwrap();
                self.execute_prompt(prompt);
            }
            Key::Backspace => {
                let prompt = self.state.prompt.as_mut().unwrap();
                prompt.input.pop();
                self.dirty = true;
            }
            Key::Char(ch) if !ch.is_control() => {
                let prompt = self.state.prompt.as_mut().unwrap();
                if prompt.is_confirm() && (ch == 'y' || ch == 'Y') {
                    let prompt = self.state.prompt.take().unwrap();
                    self.execute_prompt(prompt);
                    return;
                }
                if prompt.is_confirm() && (ch == 'n' || ch == 'N') {
                    self.state.prompt = None;
                    self.dirty = true;
                    return;
                }
                let prompt = self.state.prompt.as_mut().unwrap();
                prompt.input.push(ch);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn execute_prompt(&mut self, prompt: PromptState) {
        match prompt.kind {
            PromptKind::Mkdir => {
                let name = prompt.input.trim().to_string();
                if name.is_empty() {
                    self.state
                        .set_status(StatusLevel::Error, "directory name is required");
                    self.dirty = true;
                    return;
                }
                let target = self.state.tab().path.join(&name);
                let path = self.state.tab().path.clone();
                let tab_id = self.state.tab().id;
                let load_id = {
                    let tab = self.state.tab_mut();
                    tab.load_id = tab.load_id.wrapping_add(1);
                    tab.loading = true;
                    tab.error = None;
                    tab.entries.clear();
                    tab.cursor = 0;
                    tab.load_id
                };
                self.state.load_state = Some((tab_id, load_id));
                if let Some(handle) = self.load_handle.take() {
                    handle.abort();
                }
                self.start_parent_load(&path);
                self.pending_select = Some(name);
                self.dirty = true;

                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                tokio::spawn(async move {
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
                    match vfs.read_dir(&path).await {
                        Ok(mut stream) => {
                            if tx
                                .send(DirUpdate::Ready {
                                    tab_id,
                                    load_id,
                                    path: path.clone(),
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
                                    Err(error) => {
                                        let _ = tx
                                            .send(DirUpdate::Failed {
                                                tab_id,
                                                load_id,
                                                error: error.to_string(),
                                            })
                                            .await;
                                        return;
                                    }
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
                });
            }
            PromptKind::Touch => {
                let name = prompt.input.trim().to_string();
                if name.is_empty() {
                    self.state
                        .set_status(StatusLevel::Error, "file name is required");
                    self.dirty = true;
                    return;
                }
                let target = self.state.tab().path.join(&name);
                let path = self.state.tab().path.clone();
                let tab_id = self.state.tab().id;
                let load_id = {
                    let tab = self.state.tab_mut();
                    tab.load_id = tab.load_id.wrapping_add(1);
                    tab.loading = true;
                    tab.error = None;
                    tab.entries.clear();
                    tab.cursor = 0;
                    tab.load_id
                };
                self.state.load_state = Some((tab_id, load_id));
                if let Some(handle) = self.load_handle.take() {
                    handle.abort();
                }
                self.start_parent_load(&path);
                self.pending_select = Some(name);
                self.dirty = true;

                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                tokio::spawn(async move {
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
                    match vfs.read_dir(&path).await {
                        Ok(mut stream) => {
                            if tx
                                .send(DirUpdate::Ready {
                                    tab_id,
                                    load_id,
                                    path: path.clone(),
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
                                    Err(error) => {
                                        let _ = tx
                                            .send(DirUpdate::Failed {
                                                tab_id,
                                                load_id,
                                                error: error.to_string(),
                                            })
                                            .await;
                                        return;
                                    }
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
                });
            }
            PromptKind::Rename { target } => {
                let name = prompt.input.trim().to_string();
                if name.is_empty() {
                    self.state
                        .set_status(StatusLevel::Error, "name is required");
                    self.dirty = true;
                    return;
                }
                let destination = target
                    .parent()
                    .unwrap_or_else(|| target.clone())
                    .join(&name);
                let path = self.state.tab().path.clone();
                let tab_id = self.state.tab().id;
                let load_id = {
                    let tab = self.state.tab_mut();
                    tab.load_id = tab.load_id.wrapping_add(1);
                    tab.loading = true;
                    tab.error = None;
                    tab.entries.clear();
                    tab.cursor = 0;
                    tab.load_id
                };
                self.state.load_state = Some((tab_id, load_id));
                if let Some(handle) = self.load_handle.take() {
                    handle.abort();
                }
                self.start_parent_load(&path);
                self.pending_select = Some(name);
                self.dirty = true;

                let vfs = Arc::clone(&self.vfs);
                let tx = self.dir_tx.clone();
                tokio::spawn(async move {
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
                    match vfs.read_dir(&path).await {
                        Ok(mut stream) => {
                            if tx
                                .send(DirUpdate::Ready {
                                    tab_id,
                                    load_id,
                                    path: path.clone(),
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
                                    Err(error) => {
                                        let _ = tx
                                            .send(DirUpdate::Failed {
                                                tab_id,
                                                load_id,
                                                error: error.to_string(),
                                            })
                                            .await;
                                        return;
                                    }
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
                });
            }
            PromptKind::Filter => {
                self.state.tab_mut().filter = if prompt.input.is_empty() {
                    None
                } else {
                    Some(prompt.input)
                };
                self.dirty = true;
            }
            PromptKind::Confirm { action } => match action {
                crate::state::ConfirmAction::Delete { paths, mode } => {
                    let tab_id = self.state.tab().id;
                    let load_id = {
                        let tab = self.state.tab_mut();
                        tab.load_id = tab.load_id.wrapping_add(1);
                        tab.loading = true;
                        tab.error = None;
                        tab.entries.clear();
                        tab.cursor = 0;
                        tab.marks.clear();
                        tab.load_id
                    };
                    self.state.load_state = Some((tab_id, load_id));
                    if let Some(handle) = self.load_handle.take() {
                        handle.abort();
                    }
                    let path = self.state.tab().path.clone();
                    self.start_parent_load(&path);
                    self.pending_select = None;
                    self.dirty = true;

                    let vfs = Arc::clone(&self.vfs);
                    let tx = self.dir_tx.clone();
                    tokio::spawn(async move {
                        for target in paths {
                            let _ = vfs.remove(&target, mode).await;
                        }
                        match vfs.read_dir(&path).await {
                            Ok(mut stream) => {
                                if tx
                                    .send(DirUpdate::Ready {
                                        tab_id,
                                        load_id,
                                        path: path.clone(),
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
                                        Err(error) => {
                                            let _ = tx
                                                .send(DirUpdate::Failed {
                                                    tab_id,
                                                    load_id,
                                                    error: error.to_string(),
                                                })
                                                .await;
                                            return;
                                        }
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
                    });
                }
            },
        }
        self.dirty = true;
    }

    fn dispatch(&mut self, name: &str) {
        let effect = {
            let mut ctx = CommandContext {
                state: &mut self.state,
            };
            self.registry.dispatch(name, &mut ctx, &Args::empty())
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
                self.state.prompt = Some(prompt);
                self.dirty = true;
            }
            Effect::ToggleHidden => {
                self.state.settings.show_hidden = !self.state.settings.show_hidden;
                self.dirty = true;
            }
            Effect::CycleSort => {
                self.state.settings.sort.mode = self.state.settings.sort.mode.next();
                self.resort_current();
            }
            Effect::ToggleSortReverse => {
                self.state.settings.sort.reverse = !self.state.settings.sort.reverse;
                self.resort_current();
            }
            Effect::ToggleDirsFirst => {
                self.state.settings.sort.dirs_first = !self.state.settings.sort.dirs_first;
                self.resort_current();
            }
            Effect::SetFilter(filter) => {
                self.state.tab_mut().filter = filter;
                self.dirty = true;
            }
            Effect::CopySelection { cut } => self.copy_selection(cut),
            Effect::PasteClipboard => self.paste_clipboard(),
            Effect::DeleteSelection { mode } => self.delete_selection(mode),
            Effect::OpenExternal => self.open_external(),
            Effect::SelectAll => {
                let len = self.state.tab().entries.len();
                self.state.tab_mut().marks = (0..len).collect();
                self.dirty = true;
            }
            Effect::ClearMarks => {
                self.state.tab_mut().marks.clear();
                self.dirty = true;
            }
            Effect::ToggleMark => {
                let cursor = self.state.tab().cursor;
                let marks = &mut self.state.tab_mut().marks;
                if !marks.remove(&cursor) {
                    marks.insert(cursor);
                }
                self.dirty = true;
            }
        }
    }

    fn start_load(&mut self, path: VPath, select: Option<String>) {
        let tab_id = self.state.tab().id;
        let load_id = {
            let tab = self.state.tab_mut();
            tab.load_id = tab.load_id.wrapping_add(1);
            tab.loading = true;
            tab.error = None;
            tab.entries.clear();
            tab.cursor = 0;
            tab.marks.clear();
            tab.filter = None;
            tab.load_id
        };
        self.state.load_state = Some((tab_id, load_id));

        if let Some(handle) = self.load_handle.take() {
            handle.abort();
        }

        self.pending_select = select;
        self.state.pending_keys.clear();
        self.start_parent_load(&path);
        self.dirty = true;

        let vfs = Arc::clone(&self.vfs);
        let tx = self.dir_tx.clone();
        let settings = self.state.settings;
        self.load_handle = Some(tokio::spawn(async move {
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
                            Err(error) => {
                                let _ = tx
                                    .send(DirUpdate::Failed {
                                        tab_id,
                                        load_id,
                                        error: error.to_string(),
                                    })
                                    .await;
                                return;
                            }
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
            let _ = settings;
        }));
    }

    fn apply_dir_update(&mut self, update: DirUpdate) {
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
                let tab = &mut self.state.tabs[index];
                let hovered = if tab.cursor > 0 {
                    tab.selected().map(|entry| entry.path.clone())
                } else {
                    None
                };

                sort_entries(&mut tab.entries, self.state.settings.sort);
                tab.loading = false;

                if let Some(name) = self.pending_select.take() {
                    tab.cursor = tab
                        .entries
                        .iter()
                        .position(|entry| entry.name == name)
                        .unwrap_or(0);
                } else if let Some(path) = hovered
                    && let Some(index) = tab.entries.iter().position(|entry| entry.path == path)
                {
                    tab.cursor = index;
                }

                tab.clamp_cursor();
                self.dirty = true;
            }
            DirUpdate::Failed { error, .. } => {
                let tab = &mut self.state.tabs[index];
                tab.loading = false;
                tab.error = Some(error.clone());
                self.pending_select = None;
                self.state.set_status(StatusLevel::Error, error);
                self.dirty = true;
            }
        }
    }

    fn sync_preview(&mut self) {
        if self.state.tab().loading {
            return;
        }

        let target = self.state.tab().selected().map(|entry| entry.path.clone());
        if target == self.state.preview.path {
            return;
        }

        let Some(entry) = self.state.tab().selected().cloned() else {
            self.invalidate_preview();
            return;
        };

        self.start_preview(entry);
    }

    fn open_tab(&mut self) {
        self.state.open_tab();
        let path = self.state.tab().path.clone();
        self.state.pending_keys.clear();
        self.invalidate_preview();
        self.start_load(path, None);
        self.dirty = true;
    }

    fn close_tab(&mut self) {
        if !self.state.close_active_tab() {
            self.state
                .set_status(StatusLevel::Info, "cannot close the last tab");
            self.dirty = true;
            return;
        }

        self.pending_select = None;
        self.state.pending_keys.clear();
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

        if let Some(handle) = self.load_handle.take() {
            handle.abort();
        }
        self.pending_select = None;
        self.state.active = index;
        self.state.pending_keys.clear();
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
        self.parent_handle = Some(tokio::spawn(async move {
            let (entries, error) = match load_directory_preview(vfs, &parent_path).await {
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
        self.preview_handle = Some(tokio::spawn(async move {
            let content = load_preview(vfs, entry).await;
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

async fn load_preview(vfs: Arc<dyn Vfs>, entry: Entry) -> Preview {
    match entry.kind {
        hwe_vfs::EntryKind::Dir => load_directory_preview(vfs, &entry.path).await,
        hwe_vfs::EntryKind::File => load_text_preview(vfs, &entry).await,
        hwe_vfs::EntryKind::Other => Preview::Other,
    }
}

async fn load_directory_preview(vfs: Arc<dyn Vfs>, path: &VPath) -> Preview {
    let mut stream = match vfs.read_dir(path).await {
        Ok(stream) => stream,
        Err(error) => return Preview::Error(error.to_string()),
    };

    let mut entries = Vec::new();
    while entries.len() < MAX_PREVIEW_ENTRIES {
        match stream.next().await {
            Some(Ok(item)) => entries.push(Entry::from(item)),
            Some(Err(error)) => return Preview::Error(error.to_string()),
            None => break,
        }
    }

    let truncated = entries.len() == MAX_PREVIEW_ENTRIES && stream.next().await.is_some();
    sort_entries(&mut entries, SortSettings::default());
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

    if bytes.contains(&0) {
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

    use super::{App, Renderer, load_preview, mpsc};
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
                        .tab()
                        .entries
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
    async fn previews_text_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("note.txt");
        std::fs::write(&path, b"hello\nworld\n").unwrap();

        let preview = load_preview(Arc::new(LocalFs::new()), preview_entry(&path)).await;
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

        let preview = load_preview(Arc::new(LocalFs::new()), preview_entry(&path)).await;
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

        let preview = load_preview(Arc::new(LocalFs::new()), preview_entry(&sub)).await;
        match preview {
            Preview::Directory { entries, truncated } => {
                let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
                assert_eq!(names, ["aaa.txt", "inner.txt"]);
                assert!(!truncated);
            }
            other => panic!("unexpected preview: {other:?}"),
        }
    }
}
