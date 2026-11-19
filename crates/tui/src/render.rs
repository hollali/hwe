use std::path::PathBuf;
use std::time::SystemTime;

use hwe_core::Renderer;
use hwe_core::state::{AppState, Entry, Preview, StatusLevel};
use hwe_vfs::EntryKind;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use unicode_width::UnicodeWidthChar;

const SIZE_WIDTH: usize = 9;
const AGE_WIDTH: usize = 4;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
const HINTS: [(&str, &str); 5] = [
    ("j/k", "move"),
    ("h/l", "parent/enter"),
    ("gg/G", "top/bottom"),
    ("F5", "refresh"),
    ("q", "quit"),
];

pub struct TuiRenderer {
    terminal: DefaultTerminal,
    theme: Theme,
    scroll: usize,
    frames: u64,
}

impl TuiRenderer {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            terminal: ratatui::try_init()?,
            theme: Theme::detect(),
            scroll: 0,
            frames: 0,
        })
    }
}

impl Drop for TuiRenderer {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

impl Renderer for TuiRenderer {
    fn draw(&mut self, state: &AppState) -> std::io::Result<()> {
        let Self {
            terminal,
            theme,
            scroll,
            frames,
        } = self;
        let frame_index = frames.wrapping_add(1);
        *frames = frame_index;

        terminal.draw(|frame| render(frame, state, theme, scroll, frame_index))?;
        Ok(())
    }
}

fn render(
    frame: &mut Frame<'_>,
    state: &AppState,
    theme: &Theme,
    scroll: &mut usize,
    frame_index: u64,
) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(frame, chunks[0], state, theme, frame_index);
    draw_body(frame, chunks[1], state, scroll, theme, frame_index);
    draw_footer(frame, chunks[2], state, theme);
}

fn draw_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    scroll: &mut usize,
    theme: &Theme,
    frame_index: u64,
) {
    if area.width < 76 || area.height == 0 {
        draw_list(frame, area, state, scroll, theme, frame_index);
        return;
    }

    let chunks =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).split(area);

    draw_list(frame, chunks[0], state, scroll, theme, frame_index);
    draw_preview(frame, chunks[1], state, theme, frame_index);
}

fn draw_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    frame_index: u64,
) {
    if area.width < 16 || area.height == 0 {
        return;
    }

    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(theme.divider_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let Some(entry) = state.tab.selected() else {
        let text = if state.tab.loading {
            format!("{} loading...", spinner(theme, frame_index))
        } else {
            "no selection".to_string()
        };
        centered(frame, inner, text, theme.meta_style());
        return;
    };

    let suffix = if entry.is_symlink {
        "@"
    } else if entry.kind == EntryKind::Dir {
        "/"
    } else {
        ""
    };
    let name = format!(" {}{}", entry.name, suffix);

    let (right_text, right_style) = match &state.preview.content {
        Preview::Directory { entries, truncated } => {
            let plus = if *truncated { "+" } else { "" };
            (
                format!("{}{plus} entries  ", entries.len()),
                theme.meta_style(),
            )
        }
        Preview::Text { .. } | Preview::Binary { .. } => (
            entry.size.map(format_size).unwrap_or_default(),
            theme.meta_style(),
        ),
        _ => (format!("{}  ", kind_label(entry)), theme.meta_style()),
    };

    let right_len = right_text.chars().count() as u16;
    let title_area = Rect { height: 1, ..inner };
    let (name_area, meta_area) = if inner.width > right_len + 10 {
        let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(right_len)])
            .split(title_area);
        (chunks[0], Some(chunks[1]))
    } else {
        (title_area, None)
    };

    frame.render_widget(
        Paragraph::new(Span::styled(name, theme.path_style())),
        name_area,
    );
    if let Some(meta_area) = meta_area {
        frame.render_widget(
            Paragraph::new(right_text)
                .alignment(Alignment::Right)
                .style(right_style),
            meta_area,
        );
    }

    if inner.height < 2 {
        return;
    }

    let content_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height - 1,
    };

    match &state.preview.content {
        Preview::None => {
            let text = if state.tab.loading {
                format!("{} loading...", spinner(theme, frame_index))
            } else {
                "no preview".to_string()
            };
            centered(frame, content_area, text, theme.meta_style());
        }
        Preview::Loading => centered(
            frame,
            content_area,
            format!("{} loading...", spinner(theme, frame_index)),
            theme.meta_style(),
        ),
        Preview::Directory { entries, truncated } => {
            draw_directory_preview(frame, content_area, entries, *truncated, theme);
        }
        Preview::Text { lines, truncated } => {
            if lines.is_empty() {
                centered(
                    frame,
                    content_area,
                    "empty file".to_string(),
                    theme.meta_style(),
                );
            } else {
                let reserved = usize::from(*truncated);
                let rows = (content_area.height as usize).saturating_sub(reserved);
                let visible: Vec<Line<'_>> = lines
                    .iter()
                    .take(rows)
                    .map(|line| Line::from(line.clone()))
                    .collect();
                frame.render_widget(Paragraph::new(visible), content_area);
                if *truncated {
                    draw_more(frame, content_area, theme);
                }
            }
        }
        Preview::Binary { size } => {
            let text = match size {
                Some(size) => format!("binary file{} {}", theme.separator(), format_size(*size)),
                None => "binary file".to_string(),
            };
            centered(frame, content_area, text, theme.meta_style());
        }
        Preview::Other => centered(
            frame,
            content_area,
            "no preview available".to_string(),
            theme.meta_style(),
        ),
        Preview::Error(message) => {
            frame.render_widget(
                Paragraph::new(message.clone())
                    .style(theme.error_style())
                    .wrap(Wrap { trim: true }),
                content_area,
            );
        }
    }
}

fn draw_directory_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[Entry],
    truncated: bool,
    theme: &Theme,
) {
    if entries.is_empty() {
        centered(
            frame,
            area,
            "empty directory".to_string(),
            theme.meta_style(),
        );
        return;
    }

    let reserved = usize::from(truncated);
    let rows = (area.height as usize).saturating_sub(reserved);
    let lines: Vec<Line<'_>> = entries
        .iter()
        .take(rows)
        .map(|entry| entry_line(entry, area.width as usize, theme))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);

    if truncated {
        draw_more(frame, area, theme);
    }
}

fn draw_more(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let last = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(format!(" {} more", theme.ellipsis())).style(theme.meta_style()),
        last,
    );
}

fn centered(frame: &mut Frame<'_>, area: Rect, text: String, style: Style) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let line_area = Rect {
        x: area.x,
        y: area.y + area.height / 2,
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(style),
        line_area,
    );
}

fn kind_label(entry: &Entry) -> &'static str {
    if entry.is_symlink {
        "symlink"
    } else if entry.kind == EntryKind::Dir {
        "directory"
    } else if entry.is_executable {
        "executable"
    } else if entry.kind == EntryKind::File {
        "file"
    } else {
        "special"
    }
}

fn draw_header(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    frame_index: u64,
) {
    if area.height == 0 {
        return;
    }

    frame.render_widget(Paragraph::new("").style(theme.bar_style()), area);

    let right_text = if state.tab.loading {
        format!("{} loading  ", spinner(theme, frame_index))
    } else {
        let count = state.tab.entries.len();
        let noun = if count == 1 { "entry" } else { "entries" };
        format!("{count} {noun}  ")
    };
    let right_width = right_text.chars().count() as u16;

    let (left_area, right_area) = if area.width > right_width + 24 {
        let chunks =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let path = format!("  {}", display_path(&state.tab.path));
    frame.render_widget(
        Paragraph::new(Span::styled(path, theme.path_style())).style(theme.bar_style()),
        left_area,
    );

    if let Some(right_area) = right_area {
        let style = if state.tab.loading {
            theme.bar_style().fg(theme.info)
        } else {
            theme.bar_style().fg(theme.metadata)
        };
        frame.render_widget(
            Paragraph::new(right_text)
                .alignment(Alignment::Right)
                .style(style),
            right_area,
        );
    }
}

fn draw_list(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    scroll: &mut usize,
    theme: &Theme,
    frame_index: u64,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let tab = &state.tab;
    let height = area.height as usize;

    if tab.entries.is_empty() {
        let (message, style) = if let Some(error) = &tab.error {
            (error.clone(), theme.error_style())
        } else if tab.loading {
            (
                format!("{} loading...", spinner(theme, frame_index)),
                theme.meta_style(),
            )
        } else {
            ("empty directory".to_string(), theme.meta_style())
        };
        let line_area = Rect {
            x: area.x,
            y: area.y + area.height / 2,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(message)
                .alignment(Alignment::Center)
                .style(style),
            line_area,
        );
        return;
    }

    let max_scroll = tab.entries.len().saturating_sub(height);
    *scroll = (*scroll).min(max_scroll);
    if tab.cursor < *scroll {
        *scroll = tab.cursor;
    } else if tab.cursor >= *scroll + height {
        *scroll = tab.cursor + 1 - height;
    }
    *scroll = (*scroll).min(max_scroll);

    let show_scrollbar = tab.entries.len() > height && area.width > 16;
    let (list_area, scrollbar_area) = if show_scrollbar {
        let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let start = *scroll;
    let end = (start + height).min(tab.entries.len());
    let content_width = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem<'_>> = tab.entries[start..end]
        .iter()
        .map(|entry| ListItem::new(entry_line(entry, content_width, theme)))
        .collect();

    let list = List::new(items)
        .highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol());
    let mut list_state = ListState::default().with_selected(Some(tab.cursor - start));
    frame.render_stateful_widget(list, list_area, &mut list_state);

    if let Some(scrollbar_area) = scrollbar_area {
        let mut scrollbar_state = ScrollbarState::new(tab.entries.len()).position(tab.cursor);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(theme.scrollbar_track()))
            .thumb_symbol(theme.scrollbar_thumb())
            .track_style(Style::new().fg(theme.scroll_track))
            .thumb_style(Style::new().fg(theme.scroll_thumb));
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    frame.render_widget(Paragraph::new("").style(theme.bar_style()), area);

    let brand = format!(" fm {} ", env!("CARGO_PKG_VERSION"));
    let brand_width = brand.chars().count() as u16;
    let (left_area, brand_area) = if area.width > brand_width + 20 {
        let chunks =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(brand_width)]).split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let line = if let Some(status) = &state.status {
        let style = match status.level {
            StatusLevel::Info => theme.info_style(),
            StatusLevel::Error => theme.error_style(),
        };
        Line::from(vec![
            Span::raw(" "),
            Span::styled(status.text.clone(), style),
        ])
    } else if !state.pending_keys.is_empty() {
        let keys: Vec<String> = state.pending_keys.iter().map(ToString::to_string).collect();
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{}-", keys.join(" ")), theme.key_style()),
        ])
    } else {
        hint_line(theme, left_area.width as usize)
    };

    frame.render_widget(Paragraph::new(line).style(theme.bar_style()), left_area);

    if let Some(brand_area) = brand_area {
        frame.render_widget(
            Paragraph::new(Span::styled(brand, theme.brand_style()))
                .alignment(Alignment::Right)
                .style(theme.bar_style()),
            brand_area,
        );
    }
}

fn hint_line(theme: &Theme, width: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1usize;

    for (index, (keys, description)) in HINTS.iter().enumerate() {
        let separator = if index > 0 { 3 } else { 0 };
        let needed = separator + keys.len() + 1 + description.len();
        if used + needed > width {
            break;
        }
        if index > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(*keys, theme.key_style()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*description, theme.hint_style()));
        used += needed;
    }

    Line::from(spans)
}

fn entry_line(entry: &Entry, width: usize, theme: &Theme) -> Line<'static> {
    let suffix = if entry.is_symlink {
        "@"
    } else if entry.kind == EntryKind::Dir {
        "/"
    } else {
        ""
    };

    let name_style = if entry.is_hidden {
        theme.hidden_style()
    } else if entry.is_symlink {
        theme.symlink_style()
    } else if entry.kind == EntryKind::Dir {
        theme.dir_style()
    } else if entry.is_executable {
        theme.exec_style()
    } else {
        Style::new()
    };

    let size_text = if entry.kind == EntryKind::File {
        entry.size.map(format_size).unwrap_or_default()
    } else {
        String::new()
    };
    let age_text = format_age(entry.modified);

    let (label_width, show_size, show_age) = if width >= SIZE_WIDTH + AGE_WIDTH + 23 {
        (width - SIZE_WIDTH - AGE_WIDTH - 2, true, true)
    } else if width >= SIZE_WIDTH + 19 {
        (width - SIZE_WIDTH - 1, true, false)
    } else {
        (width, false, false)
    };

    let label = truncate_pad(
        &format!("{}{}", entry.name, suffix),
        label_width,
        theme.ellipsis(),
    );
    let mut spans = Vec::with_capacity(3);
    spans.push(Span::styled(label, name_style));
    if show_size {
        spans.push(Span::styled(
            format!(" {size_text:>SIZE_WIDTH$}"),
            theme.meta_style(),
        ));
    }
    if show_age {
        spans.push(Span::styled(
            format!(" {age_text:>AGE_WIDTH$}"),
            theme.meta_style(),
        ));
    }

    Line::from(spans)
}

fn truncate_pad(text: &str, width: usize, ellipsis: char) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;

    for ch in text.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + char_width > width {
            while used > 0 && used >= width {
                if let Some(removed) = out.pop() {
                    used = used.saturating_sub(UnicodeWidthChar::width(removed).unwrap_or(0));
                } else {
                    break;
                }
            }
            if used < width {
                out.push(ellipsis);
                used += 1;
            }
            break;
        }
        out.push(ch);
        used += char_width;
    }

    out.push_str(&" ".repeat(width - used));
    out
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    format!("{value:.1} {}", UNITS[unit])
}

fn format_age(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::new();
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return "now".to_string();
    };

    let seconds = elapsed.as_secs();
    match seconds {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        86_400..=604_799 => format!("{}d", seconds / 86_400),
        604_800..=31_535_999 => format!("{}w", seconds / 604_800),
        _ => format!("{}y", seconds / 31_536_000),
    }
}

fn display_path(path: &hwe_vfs::VPath) -> String {
    let Some(home) = home_dir() else {
        return path.display().to_string();
    };
    let Ok(rest) = path.as_path().strip_prefix(&home) else {
        return path.display().to_string();
    };

    if rest.as_os_str().is_empty() {
        "~".to_string()
    } else {
        format!("~/{}", rest.display())
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn spinner(theme: &Theme, frame_index: u64) -> &'static str {
    if theme.ascii {
        ASCII_SPINNER[(frame_index as usize / 4) % ASCII_SPINNER.len()]
    } else {
        SPINNER[(frame_index as usize / 4) % SPINNER.len()]
    }
}

#[derive(Clone, Copy)]
struct Theme {
    plain: bool,
    ascii: bool,
    accent: Color,
    dir: Color,
    exec: Color,
    symlink: Color,
    hidden: Color,
    metadata: Color,
    bar_bg: Color,
    bar_fg: Color,
    selection_bg: Color,
    selection_fg: Color,
    info: Color,
    error: Color,
    scroll_track: Color,
    scroll_thumb: Color,
}

impl Theme {
    fn detect() -> Self {
        let ascii = std::env::var_os("FM_ASCII").is_some()
            || std::env::var("TERM").is_ok_and(|term| term == "dumb");
        let plain = std::env::var_os("NO_COLOR").is_some();

        if plain {
            return Self {
                plain,
                ascii,
                accent: Color::Reset,
                dir: Color::Reset,
                exec: Color::Reset,
                symlink: Color::Reset,
                hidden: Color::Reset,
                metadata: Color::Reset,
                bar_bg: Color::Reset,
                bar_fg: Color::Reset,
                selection_bg: Color::Reset,
                selection_fg: Color::Reset,
                info: Color::Reset,
                error: Color::Reset,
                scroll_track: Color::Reset,
                scroll_thumb: Color::Reset,
            };
        }

        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|value| value.contains("truecolor") || value.contains("24bit"))
            || std::env::var("TERM").is_ok_and(|term| term.contains("direct"));

        if truecolor {
            Self {
                plain,
                ascii,
                accent: Color::Rgb(0x7a, 0xa2, 0xf7),
                dir: Color::Rgb(0x7a, 0xa2, 0xf7),
                exec: Color::Rgb(0x9e, 0xce, 0x6a),
                symlink: Color::Rgb(0xbb, 0x9a, 0xf7),
                hidden: Color::Rgb(0x56, 0x5f, 0x89),
                metadata: Color::Rgb(0x56, 0x5f, 0x89),
                bar_bg: Color::Rgb(0x16, 0x16, 0x1e),
                bar_fg: Color::Rgb(0xc0, 0xca, 0xf5),
                selection_bg: Color::Rgb(0x28, 0x34, 0x57),
                selection_fg: Color::Rgb(0xc0, 0xca, 0xf5),
                info: Color::Rgb(0x7d, 0xcf, 0xff),
                error: Color::Rgb(0xf7, 0x76, 0x8e),
                scroll_track: Color::Rgb(0x1f, 0x23, 0x35),
                scroll_thumb: Color::Rgb(0x3b, 0x42, 0x61),
            }
        } else {
            Self {
                plain,
                ascii,
                accent: Color::Indexed(111),
                dir: Color::Indexed(111),
                exec: Color::Indexed(114),
                symlink: Color::Indexed(141),
                hidden: Color::Indexed(60),
                metadata: Color::Indexed(60),
                bar_bg: Color::Indexed(234),
                bar_fg: Color::Indexed(252),
                selection_bg: Color::Indexed(24),
                selection_fg: Color::Indexed(252),
                info: Color::Indexed(117),
                error: Color::Indexed(203),
                scroll_track: Color::Indexed(236),
                scroll_thumb: Color::Indexed(60),
            }
        }
    }

    fn bar_style(&self) -> Style {
        if self.plain {
            Style::new()
        } else {
            Style::new().bg(self.bar_bg).fg(self.bar_fg)
        }
    }

    fn path_style(&self) -> Style {
        let style = Style::new().add_modifier(Modifier::BOLD);
        if self.plain {
            style
        } else {
            style.fg(self.accent)
        }
    }

    fn brand_style(&self) -> Style {
        self.path_style()
    }

    fn key_style(&self) -> Style {
        self.path_style()
    }

    fn hint_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.bar_fg)
        }
    }

    fn selection_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
                .bg(self.selection_bg)
                .fg(self.selection_fg)
                .add_modifier(Modifier::BOLD)
        }
    }

    fn dir_style(&self) -> Style {
        let style = Style::new().add_modifier(Modifier::BOLD);
        if self.plain {
            style
        } else {
            style.fg(self.dir)
        }
    }

    fn exec_style(&self) -> Style {
        let style = Style::new().add_modifier(Modifier::BOLD);
        if self.plain {
            style
        } else {
            style.fg(self.exec)
        }
    }

    fn symlink_style(&self) -> Style {
        if self.plain {
            Style::new()
        } else {
            Style::new().fg(self.symlink)
        }
    }

    fn hidden_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.hidden)
        }
    }

    fn meta_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.metadata)
        }
    }

    fn info_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.info)
        }
    }

    fn error_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.error)
        }
    }

    fn divider_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.scroll_thumb)
        }
    }

    fn separator(&self) -> &'static str {
        if self.ascii { " - " } else { " · " }
    }

    fn highlight_symbol(&self) -> &'static str {
        if self.ascii { "> " } else { "▌ " }
    }

    fn ellipsis(&self) -> char {
        if self.ascii { '~' } else { '…' }
    }

    fn scrollbar_track(&self) -> &'static str {
        if self.ascii { "|" } else { "│" }
    }

    fn scrollbar_thumb(&self) -> &'static str {
        if self.ascii { "#" } else { "┃" }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn truncation_respects_width_and_pads() {
        assert_eq!(truncate_pad("abc", 5, '…'), "abc  ");
        assert_eq!(truncate_pad("abcdef", 4, '…'), "abc…");
        assert_eq!(truncate_pad("abc", 0, '…'), "");
        assert_eq!(truncate_pad("abcdef", 4, '~'), "abc~");
    }

    #[test]
    fn truncation_handles_wide_characters() {
        assert_eq!(truncate_pad("日本", 4, '…'), "日本");
        assert_eq!(truncate_pad("日本語", 4, '…'), "日… ");
    }

    #[test]
    fn ages_are_compact() {
        let now = SystemTime::now();
        assert_eq!(format_age(None), "");
        assert_eq!(format_age(Some(now)), "now");
        assert_eq!(format_age(Some(now - Duration::from_secs(90))), "1m");
        assert_eq!(format_age(Some(now - Duration::from_secs(3 * 3600))), "3h");
        assert_eq!(
            format_age(Some(now - Duration::from_secs(2 * 86_400))),
            "2d"
        );
    }

    #[test]
    fn home_paths_are_shortened() {
        let Some(home) = home_dir() else {
            return;
        };
        let child = Path::new(&home).join("projects");
        assert_eq!(display_path(&hwe_vfs::VPath::new(&child)), "~/projects");
        assert_eq!(display_path(&hwe_vfs::VPath::new(&home)), "~");
    }
}
