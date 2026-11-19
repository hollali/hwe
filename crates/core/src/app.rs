use std::sync::Arc;
use std::time::Duration;

use hwe_vfs::{DirEntry as VfsDirEntry, VPath, Vfs};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::command::{Args, CommandContext, CommandRegistry};
use crate::commands;
use crate::effect::Effect;
use crate::event::AppEvent;
use crate::keymap::{Keymap, Resolution};
use crate::state::{AppState, Entry, Preview, PreviewState, StatusLevel, sort_entries};

const FRAME_INTERVAL: Duration = Duration::from_millis(16);
const MAX_PREVIEW_ENTRIES: usize = 1000;
const MAX_PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_PREVIEW_LINES: usize = 400;

pub trait Renderer {
    fn draw(&mut self, state: &AppState) -> std::io::Result<()>;
}

enum DirUpdate {
    Ready { id: u64, path: VPath },
    Entry { id: u64, entry: VfsDirEntry },
    Finished { id: u64 },
    Failed { id: u64, error: String },
}

struct PreviewUpdate {
    id: u64,
    content: Preview,
}

pub struct App {
    state: AppState,
    registry: CommandRegistry,
    keymap: Keymap,
    vfs: Arc<dyn Vfs>,
    events: mpsc::UnboundedReceiver<AppEvent>,
    dir_tx: mpsc::UnboundedSender<DirUpdate>,
    dir_rx: mpsc::UnboundedReceiver<DirUpdate>,
    load_handle: Option<JoinHandle<()>>,
    load_id: u64,
    preview_tx: mpsc::UnboundedSender<PreviewUpdate>,
    preview_rx: mpsc::UnboundedReceiver<PreviewUpdate>,
    preview_handle: Option<JoinHandle<()>>,
    preview_id: u64,
    pending_select: Option<String>,
    dirty: bool,
    quit: bool,
}

impl App {
    pub fn new(vfs: Arc<dyn Vfs>, start: VPath, events: mpsc::UnboundedReceiver<AppEvent>) -> Self {
        let (dir_tx, dir_rx) = mpsc::unbounded_channel();
        let (preview_tx, preview_rx) = mpsc::unbounded_channel();
        Self {
            state: AppState::new(start),
            registry: commands::registry(),
            keymap: Keymap::vim(),
            vfs,
            events,
            dir_tx,
            dir_rx,
            load_handle: None,
            load_id: 0,
            preview_tx,
            preview_rx,
            preview_handle: None,
            preview_id: 0,
            pending_select: None,
            dirty: true,
            quit: false,
        }
    }

    pub async fn run(mut self, renderer: &mut dyn Renderer) -> std::io::Result<()> {
        let start = self.state.tab.path.clone();
        self.start_load(start, None);

        let mut ticker = tokio::time::interval(FRAME_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                biased;
                _ = ticker.tick() => {
                    if self.state.expire_status() || self.state.tab.loading {
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

        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Key(key) => match self.keymap.resolve(key) {
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
            },
            AppEvent::Resize { .. } => self.dirty = true,
            AppEvent::Paste(_) => {}
        }
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
            Effect::Quit => self.quit = true,
        }
    }

    fn start_load(&mut self, path: VPath, select: Option<String>) {
        self.load_id = self.load_id.wrapping_add(1);
        let id = self.load_id;

        if let Some(handle) = self.load_handle.take() {
            handle.abort();
        }

        self.pending_select = select;
        self.state.tab.loading = true;
        self.state.tab.error = None;
        self.state.pending_keys.clear();
        self.dirty = true;

        let vfs = Arc::clone(&self.vfs);
        let tx = self.dir_tx.clone();
        self.load_handle = Some(tokio::spawn(async move {
            match vfs.read_dir(&path).await {
                Ok(mut stream) => {
                    if tx.send(DirUpdate::Ready { id, path }).is_err() {
                        return;
                    }
                    while let Some(item) = stream.next().await {
                        match item {
                            Ok(entry) => {
                                if tx.send(DirUpdate::Entry { id, entry }).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ = tx.send(DirUpdate::Failed {
                                    id,
                                    error: error.to_string(),
                                });
                                return;
                            }
                        }
                    }
                    let _ = tx.send(DirUpdate::Finished { id });
                }
                Err(error) => {
                    let _ = tx.send(DirUpdate::Failed {
                        id,
                        error: error.to_string(),
                    });
                }
            }
        }));
    }

    fn apply_dir_update(&mut self, update: DirUpdate) {
        match update {
            DirUpdate::Ready { id, path } if id == self.load_id => {
                self.state.tab.path = path;
                self.state.tab.entries.clear();
                self.state.tab.cursor = 0;
                self.state.tab.loading = true;
                self.state.tab.error = None;
                self.invalidate_preview();
                self.dirty = true;
            }
            DirUpdate::Entry { id, entry } if id == self.load_id => {
                self.state.tab.entries.push(Entry::from(entry));
                self.dirty = true;
            }
            DirUpdate::Finished { id } if id == self.load_id => {
                let hovered = if self.state.tab.cursor > 0 {
                    self.state.tab.selected().map(|entry| entry.path.clone())
                } else {
                    None
                };

                sort_entries(&mut self.state.tab.entries);
                self.state.tab.loading = false;

                if let Some(name) = self.pending_select.take() {
                    self.state.tab.cursor = self
                        .state
                        .tab
                        .entries
                        .iter()
                        .position(|entry| entry.name == name)
                        .unwrap_or(0);
                } else if let Some(path) = hovered
                    && let Some(index) = self
                        .state
                        .tab
                        .entries
                        .iter()
                        .position(|entry| entry.path == path)
                {
                    self.state.tab.cursor = index;
                }

                self.state.tab.clamp_cursor();
                self.dirty = true;
            }
            DirUpdate::Failed { id, error } if id == self.load_id => {
                self.state.tab.loading = false;
                self.state.tab.error = Some(error.clone());
                self.pending_select = None;
                self.state.set_status(StatusLevel::Error, error);
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn sync_preview(&mut self) {
        if self.state.tab.loading {
            return;
        }

        let target = self.state.tab.selected().map(|entry| entry.path.clone());
        if target == self.state.preview.path {
            return;
        }

        let Some(entry) = self.state.tab.selected().cloned() else {
            self.invalidate_preview();
            return;
        };

        self.start_preview(entry);
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
        };
        self.dirty = true;

        let vfs = Arc::clone(&self.vfs);
        let tx = self.preview_tx.clone();
        self.preview_handle = Some(tokio::spawn(async move {
            let content = load_preview(vfs, entry).await;
            let _ = tx.send(PreviewUpdate { id, content });
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
    sort_entries(&mut entries);
    Preview::Directory { entries, truncated }
}

async fn load_text_preview(vfs: Arc<dyn Vfs>, entry: &Entry) -> Preview {
    let reader = match vfs.open_read(&entry.path).await {
        Ok(reader) => reader,
        Err(error) => return Preview::Error(error.to_string()),
    };

    let mut reader = reader.take(MAX_PREVIEW_BYTES);
    let mut bytes = Vec::new();
    if let Err(error) = reader.read_to_end(&mut bytes).await {
        return Preview::Error(error.to_string());
    }

    if bytes.contains(&0) {
        return Preview::Binary { size: entry.size };
    }

    let text = String::from_utf8_lossy(&bytes);
    let total_lines = text.lines().count();
    let lines = text
        .lines()
        .take(MAX_PREVIEW_LINES)
        .map(sanitize_line)
        .collect();
    let truncated = bytes.len() as u64 == MAX_PREVIEW_BYTES || total_lines > MAX_PREVIEW_LINES;

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
    use crate::state::{AppState, Entry, Preview};

    struct RecordingRenderer {
        events: mpsc::UnboundedSender<AppEvent>,
        frames: Arc<Mutex<Vec<Vec<String>>>>,
        quit_sent: bool,
    }

    impl Renderer for RecordingRenderer {
        fn draw(&mut self, state: &AppState) -> io::Result<()> {
            if !state.tab.loading {
                self.frames.lock().unwrap().push(
                    state
                        .tab
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

        let app = App::new(Arc::new(LocalFs::new()), VPath::new(dir.path()), event_rx);
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
