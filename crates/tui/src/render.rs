use std::path::PathBuf;
use std::time::SystemTime;

use hwe_core::Renderer;
use hwe_core::state::{AppState, Entry, PaletteState, Preview, PromptState, StatusLevel};
use hwe_vfs::EntryKind;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SIZE_WIDTH: usize = 9;
const AGE_WIDTH: usize = 4;
const MODE_WIDTH: usize = 10;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: [&str; 4] = ["|", "/", "-", "\\"];
const HINTS: [(&str, &str); 8] = [
    ("j/k", "move"),
    ("h/l", "nav"),
    ("gg/G", "top/bottom"),
    ("/", "filter"),
    ("Tab", "preview"),
    ("F5", "refresh"),
    ("?", "help"),
    ("q", "quit"),
];
const PREVIEW_HINTS: [(&str, &str); 4] = [
    ("j/k", "scroll"),
    ("PgUp/PgDn", "page"),
    ("Tab", "back"),
    ("?", "help"),
];

pub struct TuiRenderer {
    terminal: DefaultTerminal,
    theme: Theme,
    scroll: usize,
    frames: u64,
}

impl TuiRenderer {
    pub fn new() -> std::io::Result<Self> {
        Self::with_options(None, None)
    }

    pub fn with_options(theme: Option<&str>, color: Option<&str>) -> std::io::Result<Self> {
        Ok(Self {
            terminal: ratatui::try_init()?,
            theme: Theme::detect(theme, color),
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
    fn cycle_theme(&mut self) {
        self.theme = self.theme.next();
    }

    fn toggle_icons(&mut self) {
        self.theme.icons = !self.theme.icons;
    }

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
    let tab_row = u16::from(state.tabs.len() > 1);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(tab_row),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(frame, chunks[0], state, theme);
    if tab_row > 0 {
        draw_tabs(frame, chunks[1], state, theme);
    }
    draw_body(frame, chunks[2], state, scroll, theme, frame_index);
    draw_footer(frame, chunks[3], state, theme);

    if state.help {
        draw_help(frame, area, theme);
    }
    if let Some(palette) = &state.palette {
        draw_palette(frame, area, palette, theme);
    }
    if let Some(prompt) = &state.prompt {
        draw_prompt(frame, area, prompt, theme);
    }
}

fn draw_body(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    scroll: &mut usize,
    theme: &Theme,
    frame_index: u64,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let framed = area.width >= 20 && area.height >= 6;
    let (list_title, list_meta) = list_titles(state, theme, frame_index);

    if area.width < 76 {
        let inner = if framed {
            draw_pane(frame, area, list_title, list_meta, theme, true)
        } else {
            area
        };
        draw_list_content(frame, inner, state, scroll, theme, frame_index);
        return;
    }

    let (preview_title, preview_meta) = preview_titles(state, theme);

    if area.width >= 110 && state.parent.path.is_some() {
        let (parent_title, parent_meta) = parent_titles(state, theme);
        let chunks = Layout::horizontal([
            Constraint::Fill(2),
            Constraint::Length(1),
            Constraint::Fill(3),
            Constraint::Length(1),
            Constraint::Fill(2),
        ])
        .split(area);

        let (parent_inner, list_inner, preview_inner) = if framed {
            (
                draw_pane(frame, chunks[0], parent_title, parent_meta, theme, false),
                draw_pane(frame, chunks[2], list_title, list_meta, theme, true),
                draw_pane(
                    frame,
                    chunks[4],
                    preview_title,
                    preview_meta,
                    theme,
                    state.preview.focus,
                ),
            )
        } else {
            (chunks[0], chunks[2], chunks[4])
        };

        draw_parent_content(frame, parent_inner, state, theme);
        draw_list_content(frame, list_inner, state, scroll, theme, frame_index);
        draw_preview_content(frame, preview_inner, state, theme, frame_index);
        return;
    }

    let chunks = Layout::horizontal([
        Constraint::Fill(3),
        Constraint::Length(1),
        Constraint::Fill(2),
    ])
    .split(area);

    let (list_inner, preview_inner) = if framed {
        (
            draw_pane(frame, chunks[0], list_title, list_meta, theme, true),
            draw_pane(
                frame,
                chunks[2],
                preview_title,
                preview_meta,
                theme,
                state.preview.focus,
            ),
        )
    } else {
        (chunks[0], chunks[2])
    };

    draw_list_content(frame, list_inner, state, scroll, theme, frame_index);
    draw_preview_content(frame, preview_inner, state, theme, frame_index);
}

fn draw_pane(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'_>,
    meta: Line<'_>,
    theme: &Theme,
    active: bool,
) -> Rect {
    let border_style = if active {
        theme.active_border_style()
    } else {
        theme.border_style()
    };
    let border_type = if theme.ascii {
        BorderType::Plain
    } else {
        BorderType::Rounded
    };

    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title_top(title)
        .title_top(meta.right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn list_titles(
    state: &AppState,
    theme: &Theme,
    frame_index: u64,
) -> (Line<'static>, Line<'static>) {
    let name = state
        .tab()
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| state.tab().path.display().to_string());
    let title = Line::from(Span::styled(format!(" {name} "), theme.path_style()));

    let meta = if state.tab().loading {
        Line::from(Span::styled(
            format!("{} loading ", spinner(theme, frame_index)),
            theme.info_style(),
        ))
    } else {
        let tab = state.tab();
        let visible = tab.visible_entries(&state.settings).len();
        let noun = if visible == 1 { "entry" } else { "entries" };
        let mut meta = format!("{visible} {noun}");
        if tab.filter.is_some() {
            meta.push_str(" [filtered]");
        }
        if state.settings.sort.mode != hwe_core::state::SortMode::Name
            || state.settings.sort.reverse
            || !state.settings.sort.dirs_first
        {
            meta.push_str(&format!(" sort:{}", state.settings.sort.mode.label()));
            if state.settings.sort.reverse {
                meta.push('↓');
            }
        }
        if state.settings.show_hidden {
            meta.push_str(" hidden");
        }
        let marks = tab
            .visible_entries(&state.settings)
            .iter()
            .filter(|entry| tab.is_marked(entry))
            .count();
        if marks > 0 {
            meta.push_str(&format!(" marks:{marks}"));
        }
        Line::from(Span::styled(format!("{meta} "), theme.meta_style()))
    };

    (title, meta)
}

fn parent_titles(state: &AppState, theme: &Theme) -> (Line<'static>, Line<'static>) {
    let name = state
        .parent
        .path
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| {
            state
                .parent
                .path
                .as_ref()
                .map(|path| path.display().to_string())
        })
        .unwrap_or_default();
    let title = Line::from(Span::styled(format!(" {name} "), theme.path_style()));

    let meta = if state.parent.loading {
        Line::from(Span::styled("loading ", theme.info_style()))
    } else {
        let count = state.parent.entries.len();
        let noun = if count == 1 { "entry" } else { "entries" };
        Line::from(Span::styled(format!("{count} {noun} "), theme.meta_style()))
    };

    (title, meta)
}

fn draw_parent_content(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    if state.parent.entries.is_empty() {
        let (title, message, style) = if let Some(error) = &state.parent.error {
            ("error", error.clone(), theme.error_style())
        } else if state.parent.loading {
            ("loading", "loading...".to_string(), theme.meta_style())
        } else {
            ("empty", "empty directory".to_string(), theme.meta_style())
        };
        draw_message_card(frame, area, title, &message, style, theme);
        return;
    }

    let content_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem<'_>> = state
        .parent
        .entries
        .iter()
        .map(|entry| ListItem::new(entry_line(entry, content_width, theme, false)))
        .collect();
    let list = List::new(items)
        .highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol());
    let selected = state
        .parent
        .entries
        .iter()
        .position(|entry| entry.path == state.tab().path);
    let mut list_state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn preview_titles(state: &AppState, theme: &Theme) -> (Line<'static>, Line<'static>) {
    let Some(entry) = state.selected() else {
        return (
            Line::from(Span::styled(" preview ", theme.meta_style())),
            Line::from(""),
        );
    };

    let suffix = if entry.is_symlink {
        "@"
    } else if entry.kind == EntryKind::Dir {
        "/"
    } else {
        ""
    };
    let title = Line::from(Span::styled(
        format!(" {}{} ", entry.name, suffix),
        theme.path_style(),
    ));

    let meta = match &state.preview.content {
        Preview::Directory { entries, truncated } => {
            let plus = if *truncated { "+" } else { "" };
            let noun = if entries.len() == 1 {
                "entry"
            } else {
                "entries"
            };
            format!("{}{plus} {noun} ", entries.len())
        }
        Preview::Text { .. } => entry
            .size
            .map(format_size)
            .map(|size| format!("{size} "))
            .unwrap_or_default(),
        Preview::Binary { size } => size
            .map(format_size)
            .map(|size| format!("{size} "))
            .unwrap_or_default(),
        Preview::Loading => "loading ".to_string(),
        Preview::Error(_) => "error ".to_string(),
        _ => format!("{} ", kind_label(entry)),
    };

    (title, Line::from(Span::styled(meta, theme.meta_style())))
}

fn draw_preview_content(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    theme: &Theme,
    frame_index: u64,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    if state.selected().is_none() {
        let text = if state.tab().loading {
            format!("{} loading...", spinner(theme, frame_index))
        } else {
            "no selection".to_string()
        };
        centered(frame, area, text, theme.meta_style());
        return;
    }

    match &state.preview.content {
        Preview::None => {
            let text = if state.tab().loading {
                format!("{} loading...", spinner(theme, frame_index))
            } else {
                "no preview".to_string()
            };
            centered(frame, area, text, theme.meta_style());
        }
        Preview::Loading => centered(
            frame,
            area,
            format!("{} loading...", spinner(theme, frame_index)),
            theme.meta_style(),
        ),
        Preview::Directory { entries, truncated } => {
            draw_directory_preview(
                frame,
                area,
                entries,
                *truncated,
                state.preview.scroll,
                theme,
            );
        }
        Preview::Text { lines, truncated } => {
            if lines.is_empty() {
                draw_message_card(
                    frame,
                    area,
                    "empty",
                    "empty file",
                    theme.meta_style(),
                    theme,
                );
            } else {
                let reserved = usize::from(*truncated);
                let rows = (area.height as usize).saturating_sub(reserved);
                if rows == 0 {
                    return;
                }

                let start = state.preview.scroll.min(lines.len().saturating_sub(rows));
                let number_width = lines.len().max(1).to_string().len();
                let show_numbers = lines.len() > 1 && area.width >= 30;
                let syntax = state.selected().and_then(|entry| syntax_for(&entry.name));

                let visible: Vec<Line<'static>> = lines
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(rows)
                    .map(|(index, line)| {
                        let mut spans = Vec::new();
                        if show_numbers {
                            spans.push(Span::styled(
                                format!("{:>number_width$} ", index + 1),
                                theme.comment_style(),
                            ));
                        }
                        spans.extend(highlight_text_line(line, syntax, theme));
                        Line::from(spans)
                    })
                    .collect();

                frame.render_widget(Paragraph::new(visible), area);
                if *truncated {
                    draw_more(frame, area, theme);
                }
            }
        }
        Preview::Binary { size } => {
            let text = match size {
                Some(size) => format!("binary file{} {}", theme.separator(), format_size(*size)),
                None => "binary file".to_string(),
            };
            centered(frame, area, text, theme.meta_style());
        }
        Preview::Other => centered(
            frame,
            area,
            "no preview available".to_string(),
            theme.meta_style(),
        ),
        Preview::Error(message) => {
            frame.render_widget(
                Paragraph::new(message.clone())
                    .style(theme.error_style())
                    .wrap(Wrap { trim: true }),
                area,
            );
        }
    }
}

fn draw_message_card(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    message: &str,
    style: Style,
    theme: &Theme,
) {
    if area.width < 12 || area.height < 3 {
        centered(frame, area, message.to_string(), style);
        return;
    }

    let message_width = message.chars().count() as u16;
    let width = (message_width + 6).clamp(16, area.width.saturating_sub(2));
    let height = 3;
    let card = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, card);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.border_style())
        .title_top(Line::from(Span::styled(
            format!(" {title} "),
            theme.meta_style(),
        )));
    frame.render_widget(block, card);

    let inner = Rect {
        x: card.x + 1,
        y: card.y + 1,
        width: card.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(message.to_string())
            .alignment(Alignment::Center)
            .style(style),
        inner,
    );
}

fn draw_directory_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    entries: &[Entry],
    truncated: bool,
    scroll: usize,
    theme: &Theme,
) {
    if entries.is_empty() {
        draw_message_card(
            frame,
            area,
            "empty",
            "empty directory",
            theme.meta_style(),
            theme,
        );
        return;
    }

    let reserved = usize::from(truncated);
    let rows = (area.height as usize).saturating_sub(reserved);
    if rows == 0 {
        return;
    }
    let start = scroll.min(entries.len().saturating_sub(rows));
    let lines: Vec<Line<'_>> = entries
        .iter()
        .skip(start)
        .take(rows)
        .map(|entry| entry_line(entry, area.width as usize, theme, false))
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

fn draw_tabs(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    frame.render_widget(Paragraph::new("").style(theme.bar_style()), area);
    let mut spans = vec![Span::raw(" ")];
    for (index, tab) in state.tabs.iter().enumerate() {
        let label = tab_label(&tab.path);
        let marker = if tab.loading { "…" } else { "" };
        let style = if index == state.active {
            theme.selection_style()
        } else {
            theme.hint_style()
        };
        spans.push(Span::styled(
            format!(" {} {label}{marker} ", index + 1),
            style,
        ));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.bar_style()),
        area,
    );
}

fn tab_label(path: &hwe_vfs::VPath) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, state: &AppState, theme: &Theme) {
    if area.height == 0 {
        return;
    }

    frame.render_widget(Paragraph::new("").style(theme.bar_style()), area);
    frame.render_widget(
        Paragraph::new(breadcrumb(&state.tab().path, theme)).style(theme.bar_style()),
        area,
    );
}

fn breadcrumb(path: &hwe_vfs::VPath, theme: &Theme) -> Line<'static> {
    let display = display_path(path);
    let separator = if theme.ascii { " > " } else { " › " };
    let mut spans = vec![Span::raw("  ")];
    let parts: Vec<&str> = display
        .split(std::path::MAIN_SEPARATOR)
        .filter(|part| !part.is_empty())
        .collect();

    if parts.is_empty() {
        spans.push(Span::styled("/", theme.path_style()));
        return Line::from(spans);
    }

    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(separator, theme.meta_style()));
        }
        let style = if index + 1 == parts.len() {
            theme.path_style()
        } else {
            theme.meta_style()
        };
        spans.push(Span::styled((*part).to_string(), style));
    }

    Line::from(spans)
}

fn draw_list_content(
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

    let tab = state.tab();
    let height = area.height as usize;
    let visible = tab.visible_entries(&state.settings);

    if visible.is_empty() {
        let (title, message, style) = if let Some(error) = &tab.error {
            ("error", error.clone(), theme.error_style())
        } else if tab.loading {
            (
                "loading",
                format!("{} loading...", spinner(theme, frame_index)),
                theme.meta_style(),
            )
        } else {
            ("empty", "empty directory".to_string(), theme.meta_style())
        };
        draw_message_card(frame, area, title, &message, style, theme);
        return;
    }

    if area.width < 4 {
        return;
    }

    let max_scroll = visible.len().saturating_sub(height);
    *scroll = (*scroll).min(max_scroll);
    if tab.cursor < *scroll {
        *scroll = tab.cursor;
    } else if tab.cursor >= *scroll + height {
        *scroll = tab.cursor + 1 - height;
    }
    *scroll = (*scroll).min(max_scroll);

    let show_scrollbar = visible.len() > height && area.width > 16;
    let (list_area, scrollbar_area) = if show_scrollbar {
        let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };

    let start = *scroll;
    let end = (start + height).min(visible.len());
    let content_width = list_area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem<'_>> = visible[start..end]
        .iter()
        .map(|entry| {
            let marked = tab.is_marked(entry);
            ListItem::new(entry_line(entry, content_width, theme, marked))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol());
    let mut list_state = ListState::default().with_selected(Some(tab.cursor - start));
    frame.render_stateful_widget(list, list_area, &mut list_state);

    if let Some(scrollbar_area) = scrollbar_area {
        let mut scrollbar_state = ScrollbarState::new(visible.len()).position(tab.cursor);
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

    let brand = format!(
        " fm {} {} {} ",
        env!("CARGO_PKG_VERSION"),
        theme.separator().trim(),
        theme.theme_label()
    );
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
            Span::styled(format!(" {} ", keys.join(" ")), theme.chip_style()),
            Span::raw(" "),
            Span::styled("pending", theme.hint_style()),
        ])
    } else {
        hint_line(
            theme,
            left_area.width as usize,
            if state.preview.focus {
                &PREVIEW_HINTS
            } else {
                &HINTS
            },
        )
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

fn hint_line(theme: &Theme, width: usize, hints: &[(&'static str, &'static str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let mut used = 1usize;

    for (index, (keys, description)) in hints.iter().enumerate() {
        let separator = if index > 0 { 2 } else { 0 };
        let needed = separator + keys.len() + 2 + 1 + description.len();
        if used + needed > width {
            break;
        }
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!(" {keys} "), theme.chip_style()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(*description, theme.hint_style()));
        used += needed;
    }

    Line::from(spans)
}

fn entry_line(entry: &Entry, width: usize, theme: &Theme, marked: bool) -> Line<'static> {
    let suffix = if entry.is_symlink {
        "@"
    } else if entry.kind == EntryKind::Dir {
        "/"
    } else {
        ""
    };

    let extension = extension(&entry.name);
    let name_style = if entry.is_hidden {
        theme.hidden_style()
    } else if entry.is_symlink {
        theme.symlink_style()
    } else if entry.kind == EntryKind::Dir {
        theme.dir_style()
    } else if entry.is_executable {
        theme.exec_style()
    } else {
        theme.file_extension_style(extension.as_deref())
    };

    let size_text = if entry.kind == EntryKind::File {
        entry.size.map(format_size).unwrap_or_default()
    } else {
        String::new()
    };
    let age_text = format_age(entry.modified);

    let show_mode = width >= 64 && entry.mode.is_some();
    let (label_width, show_size, show_age) = if show_mode {
        (
            width.saturating_sub(MODE_WIDTH + SIZE_WIDTH + AGE_WIDTH + 3),
            true,
            true,
        )
    } else if width >= SIZE_WIDTH + AGE_WIDTH + 23 {
        (width - SIZE_WIDTH - AGE_WIDTH - 2, true, true)
    } else if width >= SIZE_WIDTH + 19 {
        (width - SIZE_WIDTH - 1, true, false)
    } else {
        (width, false, false)
    };

    let icon = icon_for(entry, theme, extension.as_deref());
    let icon_width = icon
        .map(|(glyph, _)| UnicodeWidthStr::width(glyph))
        .unwrap_or(0);
    let use_icon = icon.is_some() && label_width > icon_width + 1;
    let name_width = label_width.saturating_sub(if use_icon { icon_width + 1 } else { 0 });
    let sanitized_name: String = entry
        .name
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let label = truncate_pad(
        &format!("{}{}", sanitized_name, suffix),
        name_width,
        theme.ellipsis(),
    );

    let mut spans = Vec::with_capacity(6);
    if marked {
        spans.push(Span::styled("*", theme.key_style()));
    }
    if use_icon && let Some((glyph, style)) = icon {
        spans.push(Span::styled(glyph, style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(label, name_style));
    if show_mode {
        spans.push(Span::styled(
            format!(" {}", format_mode(entry)),
            theme.meta_style(),
        ));
    }
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

fn extension(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() || extension.is_empty() {
        return None;
    }
    Some(extension.to_ascii_lowercase())
}

fn icon_for<'a>(entry: &Entry, theme: &Theme, extension: Option<&str>) -> Option<(&'a str, Style)> {
    if !theme.icons || theme.ascii {
        return None;
    }
    if entry.is_symlink {
        return Some(("\u{f0c1}", theme.symlink_style()));
    }
    if entry.kind == EntryKind::Dir {
        return Some(("\u{f07b}", theme.dir_style()));
    }
    if entry.is_executable {
        return Some(("\u{f120}", theme.exec_style()));
    }

    let glyph = match extension.unwrap_or("") {
        "rs" => "\u{e7a8}",
        "go" => "\u{e627}",
        "py" => "\u{e73c}",
        "js" | "mjs" | "cjs" => "\u{e74e}",
        "ts" | "tsx" => "\u{e628}",
        "json" => "\u{e60b}",
        "md" | "markdown" => "\u{e73e}",
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "env" => "\u{f013}",
        "sh" | "bash" | "zsh" | "fish" => "\u{f489}",
        "c" | "h" => "\u{e61e}",
        "cc" | "cpp" | "cxx" | "hpp" => "\u{e61d}",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "tiff" => "\u{f1c5}",
        "zip" | "tar" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "deb" | "rpm" => "\u{f1c6}",
        "mp4" | "mkv" | "webm" | "mov" | "avi" => "\u{f1c8}",
        "mp3" | "flac" | "wav" | "ogg" | "m4a" => "\u{f1c7}",
        "pdf" => "\u{f1c1}",
        "doc" | "docx" => "\u{f1c2}",
        "xls" | "xlsx" | "csv" => "\u{f1c3}",
        "ppt" | "pptx" => "\u{f1c4}",
        "txt" | "log" => "\u{f15c}",
        "lock" => "\u{f023}",
        _ => "\u{f15b}",
    };

    Some((glyph, theme.file_extension_style(extension)))
}

fn format_mode(entry: &Entry) -> String {
    let Some(mode) = entry.mode else {
        return String::new();
    };

    let kind = if entry.is_symlink {
        'l'
    } else if entry.kind == EntryKind::Dir {
        'd'
    } else if entry.kind == EntryKind::File {
        '-'
    } else {
        '?'
    };

    let mut text = String::with_capacity(10);
    text.push(kind);
    for (bit, symbol) in [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ] {
        text.push(if mode & bit != 0 { symbol } else { '-' });
    }
    text
}

struct Syntax {
    keywords: &'static [&'static str],
    line_comment: &'static str,
}

static RUST: Syntax = Syntax {
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
        "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait",
        "true", "type", "unsafe", "use", "where", "while",
    ],
    line_comment: "//",
};

static PYTHON: Syntax = Syntax {
    keywords: &[
        "and", "as", "async", "await", "break", "class", "continue", "def", "elif", "else",
        "except", "False", "finally", "for", "from", "global", "if", "import", "in", "lambda",
        "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while", "with",
        "yield",
    ],
    line_comment: "#",
};

static JAVASCRIPT: Syntax = Syntax {
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "null",
        "readonly",
        "return",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "undefined",
        "var",
        "while",
    ],
    line_comment: "//",
};

static C_LIKE: Syntax = Syntax {
    keywords: &[
        "auto",
        "break",
        "case",
        "char",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "double",
        "else",
        "enum",
        "extern",
        "false",
        "float",
        "for",
        "if",
        "int",
        "long",
        "namespace",
        "new",
        "nullptr",
        "private",
        "public",
        "return",
        "short",
        "signed",
        "sizeof",
        "static",
        "struct",
        "switch",
        "template",
        "true",
        "typedef",
        "unsigned",
        "using",
        "void",
        "while",
    ],
    line_comment: "//",
};

static GO: Syntax = Syntax {
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "false",
        "for",
        "func",
        "go",
        "if",
        "import",
        "interface",
        "map",
        "nil",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "true",
        "type",
        "var",
    ],
    line_comment: "//",
};

static SHELL: Syntax = Syntax {
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if",
        "in", "local", "return", "then", "while",
    ],
    line_comment: "#",
};

static CONFIG: Syntax = Syntax {
    keywords: &["false", "null", "true"],
    line_comment: "#",
};

static JSON_SYNTAX: Syntax = Syntax {
    keywords: &["false", "null", "true"],
    line_comment: "//",
};

fn syntax_for(name: &str) -> Option<&'static Syntax> {
    match extension(name).as_deref()? {
        "rs" => Some(&RUST),
        "py" => Some(&PYTHON),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Some(&JAVASCRIPT),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" => Some(&C_LIKE),
        "go" => Some(&GO),
        "sh" | "bash" | "zsh" | "fish" => Some(&SHELL),
        "json" => Some(&JSON_SYNTAX),
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" | "env" => Some(&CONFIG),
        _ => None,
    }
}

fn highlight_text_line(line: &str, syntax: Option<&Syntax>, theme: &Theme) -> Vec<Span<'static>> {
    let Some(syntax) = syntax else {
        return vec![Span::raw(line.to_string())];
    };

    let chars: Vec<char> = line.chars().collect();
    let comment: Vec<char> = syntax.line_comment.chars().collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index..].starts_with(&comment) {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            spans.push(Span::styled(
                chars[index..].iter().collect::<String>(),
                theme.comment_style(),
            ));
            return spans;
        }

        let ch = chars[index];
        if ch == '"' || ch == '\'' || ch == '`' {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            let start = index;
            index += 1;
            while index < chars.len() {
                let current = chars[index];
                index += 1;
                if current == '\\' && index < chars.len() {
                    index += 1;
                } else if current == ch {
                    break;
                }
            }
            spans.push(Span::styled(
                chars[start..index].iter().collect::<String>(),
                theme.string_style(),
            ));
            continue;
        }

        if ch.is_ascii_digit()
            && (index == 0 || !chars[index - 1].is_alphanumeric() && chars[index - 1] != '_')
        {
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            let start = index;
            while index < chars.len()
                && (chars[index].is_alphanumeric()
                    || matches!(chars[index], '.' | '_' | 'x' | 'X' | 'o' | 'O' | 'b' | 'B'))
            {
                index += 1;
            }
            spans.push(Span::styled(
                chars[start..index].iter().collect::<String>(),
                theme.number_style(),
            ));
            continue;
        }

        if ch.is_alphabetic() || ch == '_' {
            let start = index;
            while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            if syntax.keywords.contains(&word.as_str()) {
                if !plain.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut plain)));
                }
                spans.push(Span::styled(word, theme.keyword_style()));
            } else {
                plain.push_str(&word);
            }
            continue;
        }

        plain.push(ch);
        index += 1;
    }

    if !plain.is_empty() {
        spans.push(Span::raw(plain));
    }
    spans
}

fn help_line(keys: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(format!(" {keys} "), theme.chip_style()),
        Span::raw("  "),
        Span::styled(description.to_string(), theme.hint_style()),
    ])
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    if area.width < 32 || area.height < 12 {
        return;
    }

    let width = 64.min(area.width.saturating_sub(4));
    let height = 25.min(area.height.saturating_sub(4));
    let card = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, card);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.active_border_style())
        .title_top(Line::from(Span::styled(" keys ", theme.path_style())))
        .title_bottom(
            Line::from(Span::styled(
                " Esc closes · : commands ",
                theme.meta_style(),
            ))
            .right_aligned(),
        );
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let lines = vec![
        help_line("j / k", "move down / up", theme),
        help_line("h / l", "parent / enter directory", theme),
        help_line("gg / G", "top / bottom", theme),
        help_line("Tab", "focus the preview pane", theme),
        help_line("PgUp / PgDn", "scroll the preview", theme),
        help_line("Space/m", "toggle mark", theme),
        help_line("d / D", "delete / permanent delete", theme),
        help_line("y / x / p", "copy / cut / paste", theme),
        help_line("r / n / N", "rename / mkdir / touch", theme),
        help_line("o", "open externally", theme),
        help_line("H", "toggle hidden files", theme),
        help_line("s / S", "sort mode / reverse", theme),
        help_line("/ / \\", "filter / clear filter", theme),
        help_line("A / u", "select all / clear marks", theme),
        help_line("C-t / C-w", "new / close tab", theme),
        help_line("gt / gT", "next / previous tab", theme),
        help_line("F5", "refresh", theme),
        help_line("T", "cycle color theme", theme),
        help_line("I", "toggle file icons", theme),
        help_line(":", "command palette", theme),
        help_line("?", "toggle this help", theme),
        help_line("q / C-c", "quit", theme),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_palette(frame: &mut Frame<'_>, area: Rect, palette: &PaletteState, theme: &Theme) {
    if area.width < 40 || area.height < 8 {
        return;
    }

    let width = 72.min(area.width.saturating_sub(4));
    let max_matches = area.height.saturating_sub(6) as usize;
    let visible_matches = palette.matches.len().min(max_matches);
    let height = (visible_matches as u16 + 4).min(area.height.saturating_sub(4));
    let card = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, card);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.active_border_style())
        .title_top(Line::from(Span::styled(" commands ", theme.path_style())))
        .title_bottom(
            Line::from(Span::styled(
                " Enter runs · Esc closes ",
                theme.meta_style(),
            ))
            .right_aligned(),
        );
    let inner = block.inner(card);
    frame.render_widget(block, card);

    if inner.height == 0 {
        return;
    }

    let query_area = Rect { height: 1, ..inner };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" :", theme.key_style()),
            Span::raw(palette.query.clone()),
        ]))
        .style(theme.bar_style()),
        query_area,
    );

    let divider_area = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new("─".repeat(inner.width as usize)).style(theme.divider_style()),
        divider_area,
    );

    let list_area = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: inner.height.saturating_sub(2),
    };
    if palette.matches.is_empty() {
        frame.render_widget(
            Paragraph::new("no matching commands").style(theme.meta_style()),
            list_area,
        );
        return;
    }

    let items: Vec<ListItem<'_>> = palette
        .matches
        .iter()
        .map(|entry| {
            ListItem::new(Line::from(vec![
                Span::styled(entry.name.clone(), theme.code_style()),
                Span::raw("  "),
                Span::styled(entry.description.clone(), theme.hint_style()),
            ]))
        })
        .collect();
    let list = List::new(items)
        .highlight_style(theme.selection_style())
        .highlight_symbol(theme.highlight_symbol());
    let mut list_state = ListState::default().with_selected(Some(
        palette
            .selected
            .min(palette.matches.len().saturating_sub(1)),
    ));
    frame.render_stateful_widget(list, list_area, &mut list_state);
}

fn draw_prompt(frame: &mut Frame<'_>, area: Rect, prompt: &PromptState, theme: &Theme) {
    if area.width < 30 || area.height < 5 {
        return;
    }
    let width = 60.min(area.width.saturating_sub(4));
    let height = 5u16.min(area.height.saturating_sub(4));
    let card = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height.saturating_sub(height + 2),
        width,
        height,
    };
    frame.render_widget(Clear, card);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(if theme.ascii {
            BorderType::Plain
        } else {
            BorderType::Rounded
        })
        .border_style(theme.active_border_style())
        .title_top(Line::from(Span::styled(
            format!(" {} ", prompt.title),
            theme.path_style(),
        )))
        .title_bottom(Line::from(Span::styled(
            format!(" {} ", prompt.hint),
            theme.meta_style(),
        )));
    let inner = block.inner(card);
    frame.render_widget(block, card);

    if prompt.is_confirm() {
        let line = if prompt.input.is_empty() {
            "Press Enter/y to confirm, Esc/n to cancel".to_string()
        } else {
            prompt.input.clone()
        };
        frame.render_widget(
            Paragraph::new(line).style(theme.bar_style()),
            Rect { height: 1, ..inner },
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", theme.key_style()),
            Span::raw(prompt.input.clone()),
        ]))
        .style(theme.bar_style()),
        Rect { height: 1, ..inner },
    );
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorMode {
    TrueColor,
    Indexed,
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum ThemeName {
    #[default]
    TokyoNight,
    Catppuccin,
    Gruvbox,
    Nord,
}

impl ThemeName {
    fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        match normalized.as_str() {
            "tokyo-night" | "tokyonight" | "tokyo" => Some(Self::TokyoNight),
            "catppuccin" | "catppuccin-mocha" | "mocha" => Some(Self::Catppuccin),
            "gruvbox" | "gruvbox-dark" | "gruvboxdark" => Some(Self::Gruvbox),
            "nord" => Some(Self::Nord),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::TokyoNight => "Tokyo Night",
            Self::Catppuccin => "Catppuccin",
            Self::Gruvbox => "Gruvbox",
            Self::Nord => "Nord",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::TokyoNight => Self::Catppuccin,
            Self::Catppuccin => Self::Gruvbox,
            Self::Gruvbox => Self::Nord,
            Self::Nord => Self::TokyoNight,
        }
    }
}

type Rgb = (u8, u8, u8);

struct Palette {
    accent: Rgb,
    dir: Rgb,
    exec: Rgb,
    symlink: Rgb,
    hidden: Rgb,
    metadata: Rgb,
    bar_bg: Rgb,
    bar_fg: Rgb,
    selection_bg: Rgb,
    selection_fg: Rgb,
    info: Rgb,
    error: Rgb,
    scroll_track: Rgb,
    scroll_thumb: Rgb,
    border: Rgb,
    file: Rgb,
    code: Rgb,
    string: Rgb,
    comment: Rgb,
    number: Rgb,
    keyword: Rgb,
    archive: Rgb,
    image: Rgb,
    media: Rgb,
    config: Rgb,
}

fn palette(name: ThemeName) -> Palette {
    match name {
        ThemeName::TokyoNight => Palette {
            accent: (0x7a, 0xa2, 0xf7),
            dir: (0x7a, 0xa2, 0xf7),
            exec: (0x9e, 0xce, 0x6a),
            symlink: (0xbb, 0x9a, 0xf7),
            hidden: (0x56, 0x5f, 0x89),
            metadata: (0x56, 0x5f, 0x89),
            bar_bg: (0x16, 0x16, 0x1e),
            bar_fg: (0xc0, 0xca, 0xf5),
            selection_bg: (0x28, 0x34, 0x57),
            selection_fg: (0xc0, 0xca, 0xf5),
            info: (0x7d, 0xcf, 0xff),
            error: (0xf7, 0x76, 0x8e),
            scroll_track: (0x1f, 0x23, 0x35),
            scroll_thumb: (0x3b, 0x42, 0x61),
            border: (0x3b, 0x42, 0x61),
            file: (0xc0, 0xca, 0xf5),
            code: (0x7d, 0xcf, 0xff),
            string: (0x9e, 0xce, 0x6a),
            comment: (0x56, 0x5f, 0x89),
            number: (0xff, 0x9e, 0x64),
            keyword: (0xbb, 0x9a, 0xf7),
            archive: (0xe0, 0xaf, 0x68),
            image: (0x7d, 0xcf, 0xff),
            media: (0x9e, 0xce, 0x6a),
            config: (0xe0, 0xaf, 0x68),
        },
        ThemeName::Catppuccin => Palette {
            accent: (0x89, 0xb4, 0xfa),
            dir: (0x89, 0xb4, 0xfa),
            exec: (0xa6, 0xe3, 0xa1),
            symlink: (0xcb, 0xa6, 0xf7),
            hidden: (0x6c, 0x70, 0x86),
            metadata: (0x6c, 0x70, 0x86),
            bar_bg: (0x18, 0x18, 0x25),
            bar_fg: (0xcd, 0xd6, 0xf4),
            selection_bg: (0x31, 0x32, 0x44),
            selection_fg: (0xcd, 0xd6, 0xf4),
            info: (0x89, 0xdc, 0xeb),
            error: (0xf3, 0x8b, 0xa8),
            scroll_track: (0x31, 0x32, 0x44),
            scroll_thumb: (0x58, 0x5b, 0x70),
            border: (0x45, 0x47, 0x5a),
            file: (0xcd, 0xd6, 0xf4),
            code: (0x89, 0xdc, 0xeb),
            string: (0xa6, 0xe3, 0xa1),
            comment: (0x6c, 0x70, 0x86),
            number: (0xfa, 0xb3, 0x87),
            keyword: (0xcb, 0xa6, 0xf7),
            archive: (0xf9, 0xe2, 0xaf),
            image: (0x89, 0xdc, 0xeb),
            media: (0xa6, 0xe3, 0xa1),
            config: (0xf9, 0xe2, 0xaf),
        },
        ThemeName::Gruvbox => Palette {
            accent: (0xfe, 0x80, 0x19),
            dir: (0x83, 0xa5, 0x98),
            exec: (0xb8, 0xbb, 0x26),
            symlink: (0xd3, 0x86, 0x9b),
            hidden: (0x92, 0x83, 0x74),
            metadata: (0x92, 0x83, 0x74),
            bar_bg: (0x1d, 0x20, 0x21),
            bar_fg: (0xeb, 0xdb, 0xb2),
            selection_bg: (0x3c, 0x38, 0x36),
            selection_fg: (0xfa, 0xbd, 0x2f),
            info: (0x8e, 0xc0, 0x7c),
            error: (0xfb, 0x49, 0x34),
            scroll_track: (0x32, 0x30, 0x2f),
            scroll_thumb: (0x50, 0x49, 0x45),
            border: (0x50, 0x49, 0x45),
            file: (0xeb, 0xdb, 0xb2),
            code: (0x8e, 0xc0, 0x7c),
            string: (0xb8, 0xbb, 0x26),
            comment: (0x92, 0x83, 0x74),
            number: (0xd3, 0x86, 0x9b),
            keyword: (0xfb, 0x49, 0x34),
            archive: (0xfa, 0xbd, 0x2f),
            image: (0x8e, 0xc0, 0x7c),
            media: (0xb8, 0xbb, 0x26),
            config: (0xfa, 0xbd, 0x2f),
        },
        ThemeName::Nord => Palette {
            accent: (0x88, 0xc0, 0xd0),
            dir: (0x88, 0xc0, 0xd0),
            exec: (0xa3, 0xbe, 0x8c),
            symlink: (0xb4, 0x8e, 0xad),
            hidden: (0x61, 0x6e, 0x88),
            metadata: (0x61, 0x6e, 0x88),
            bar_bg: (0x24, 0x29, 0x33),
            bar_fg: (0xd8, 0xde, 0xe9),
            selection_bg: (0x43, 0x4c, 0x5e),
            selection_fg: (0xec, 0xef, 0xf4),
            info: (0x8f, 0xbc, 0xbb),
            error: (0xbf, 0x61, 0x6a),
            scroll_track: (0x3b, 0x42, 0x52),
            scroll_thumb: (0x4c, 0x56, 0x6a),
            border: (0x4c, 0x56, 0x6a),
            file: (0xd8, 0xde, 0xe9),
            code: (0x8f, 0xbc, 0xbb),
            string: (0xa3, 0xbe, 0x8c),
            comment: (0x61, 0x6e, 0x88),
            number: (0xb4, 0x8e, 0xad),
            keyword: (0x81, 0xa1, 0xc1),
            archive: (0xeb, 0xcb, 0x8b),
            image: (0x88, 0xc0, 0xd0),
            media: (0xa3, 0xbe, 0x8c),
            config: (0xeb, 0xcb, 0x8b),
        },
    }
}

fn color_for(rgb: Rgb, mode: ColorMode) -> Color {
    match mode {
        ColorMode::Plain => Color::Reset,
        ColorMode::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
        ColorMode::Indexed => Color::Indexed(ansi256(rgb)),
    }
}

fn ansi256((r, g, b): Rgb) -> u8 {
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return (((r as u16 - 8) * 24 / 247) + 232) as u8;
    }

    let quantize = |value: u8| -> u16 { (value as u16 * 5 + 127) / 255 };
    (16 + 36 * quantize(r) + 6 * quantize(g) + quantize(b)) as u8
}

#[derive(Clone, Copy)]
struct Theme {
    plain: bool,
    ascii: bool,
    icons: bool,
    name: ThemeName,
    mode: ColorMode,
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
    border: Color,
    file: Color,
    code: Color,
    string: Color,
    comment: Color,
    number: Color,
    keyword: Color,
    archive: Color,
    image: Color,
    media: Color,
    config: Color,
}

impl Theme {
    fn detect(theme: Option<&str>, color: Option<&str>) -> Self {
        let env_theme = std::env::var("FM_THEME").ok();
        let name = theme
            .and_then(ThemeName::parse)
            .or_else(|| env_theme.as_deref().and_then(ThemeName::parse))
            .unwrap_or_default();

        let env_color = std::env::var("FM_COLOR").ok();
        let requested = color.or(env_color.as_deref()).map(str::to_ascii_lowercase);
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let plain = no_color
            || requested
                .as_deref()
                .is_some_and(|value| matches!(value, "none" | "plain" | "no-color"));

        let mode = if plain {
            ColorMode::Plain
        } else {
            match requested.as_deref() {
                Some("truecolor" | "24bit") => ColorMode::TrueColor,
                Some("256" | "indexed") => ColorMode::Indexed,
                _ => {
                    if truecolor_detected() {
                        ColorMode::TrueColor
                    } else {
                        ColorMode::Indexed
                    }
                }
            }
        };

        let ascii = std::env::var_os("FM_ASCII").is_some()
            || std::env::var("TERM").is_ok_and(|term| term == "dumb");
        let icons = std::env::var("FM_ICONS").is_ok_and(|value| {
            !matches!(
                value.to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        });

        Self::new(name, mode, ascii, icons)
    }

    fn new(name: ThemeName, mode: ColorMode, ascii: bool, icons: bool) -> Self {
        let p = palette(name);
        let plain = mode == ColorMode::Plain;
        let adapt = |rgb: Rgb| color_for(rgb, mode);

        Self {
            plain,
            ascii,
            icons,
            name,
            mode,
            accent: adapt(p.accent),
            dir: adapt(p.dir),
            exec: adapt(p.exec),
            symlink: adapt(p.symlink),
            hidden: adapt(p.hidden),
            metadata: adapt(p.metadata),
            bar_bg: adapt(p.bar_bg),
            bar_fg: adapt(p.bar_fg),
            selection_bg: adapt(p.selection_bg),
            selection_fg: adapt(p.selection_fg),
            info: adapt(p.info),
            error: adapt(p.error),
            scroll_track: adapt(p.scroll_track),
            scroll_thumb: adapt(p.scroll_thumb),
            border: adapt(p.border),
            file: adapt(p.file),
            code: adapt(p.code),
            string: adapt(p.string),
            comment: adapt(p.comment),
            number: adapt(p.number),
            keyword: adapt(p.keyword),
            archive: adapt(p.archive),
            image: adapt(p.image),
            media: adapt(p.media),
            config: adapt(p.config),
        }
    }

    fn next(self) -> Self {
        Self::new(self.name.next(), self.mode, self.ascii, self.icons)
    }

    fn theme_label(&self) -> &'static str {
        self.name.label()
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

    fn chip_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new()
                .bg(self.selection_bg)
                .fg(self.accent)
                .add_modifier(Modifier::BOLD)
        }
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

    fn file_style(&self) -> Style {
        if self.plain {
            Style::new()
        } else {
            Style::new().fg(self.file)
        }
    }

    fn file_extension_style(&self, extension: Option<&str>) -> Style {
        if self.plain {
            return Style::new();
        }
        let Some(extension) = extension else {
            return self.file_style();
        };
        let color = match extension {
            "rs" | "go" | "c" | "h" | "cc" | "cpp" | "hpp" | "js" | "jsx" | "ts" | "tsx" | "py"
            | "rb" | "lua" | "sh" | "bash" | "zsh" | "fish" | "zig" | "swift" | "kt" | "java"
            | "cs" | "php" | "sql" | "nix" => self.code,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "tiff" => self.image,
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "mp3" | "flac" | "wav" | "ogg" | "m4a" => {
                self.media
            }
            "zip" | "tar" | "gz" | "xz" | "bz2" | "zst" | "7z" | "rar" | "deb" | "rpm" => {
                self.archive
            }
            "toml" | "yaml" | "yml" | "json" | "ini" | "conf" | "cfg" | "env" => self.config,
            _ => self.file,
        };
        Style::new().fg(color)
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

    fn border_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.border)
        }
    }

    fn active_border_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.accent)
        }
    }

    fn divider_style(&self) -> Style {
        self.border_style()
    }

    fn code_style(&self) -> Style {
        Style::new().fg(if self.plain { Color::Reset } else { self.code })
    }

    fn string_style(&self) -> Style {
        Style::new().fg(if self.plain {
            Color::Reset
        } else {
            self.string
        })
    }

    fn comment_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::DIM)
        } else {
            Style::new().fg(self.comment)
        }
    }

    fn number_style(&self) -> Style {
        Style::new().fg(if self.plain {
            Color::Reset
        } else {
            self.number
        })
    }

    fn keyword_style(&self) -> Style {
        if self.plain {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(self.keyword).add_modifier(Modifier::BOLD)
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

fn truecolor_detected() -> bool {
    std::env::var("COLORTERM")
        .is_ok_and(|value| value.contains("truecolor") || value.contains("24bit"))
        || std::env::var("TERM").is_ok_and(|term| term.contains("direct"))
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
    fn themes_parse_and_cycle() {
        assert_eq!(ThemeName::parse("nord"), Some(ThemeName::Nord));
        assert_eq!(ThemeName::parse("Tokyo_Night"), Some(ThemeName::TokyoNight));
        assert_eq!(
            ThemeName::parse("catppuccin-mocha"),
            Some(ThemeName::Catppuccin)
        );
        assert_eq!(ThemeName::parse("unknown"), None);
        assert_eq!(ThemeName::Nord.next(), ThemeName::TokyoNight);
        assert_eq!(ThemeName::TokyoNight.next(), ThemeName::Catppuccin);
    }

    #[test]
    fn plain_and_ascii_modes_are_supported() {
        let theme = Theme::new(ThemeName::Gruvbox, ColorMode::Plain, true, false);
        assert!(theme.plain);
        assert!(theme.ascii);
        assert_eq!(theme.bar_style(), Style::new());
        assert_eq!(theme.highlight_symbol(), "> ");

        let theme = Theme::new(ThemeName::Nord, ColorMode::TrueColor, false, true);
        assert!(!theme.plain);
        assert_eq!(theme.name, ThemeName::Nord);
        assert!(matches!(theme.accent, Color::Rgb(0x88, 0xc0, 0xd0)));
    }

    #[test]
    fn indexed_palette_uses_ansi_colors() {
        assert_eq!(ansi256((0, 0, 0)), 16);
        assert_eq!(ansi256((255, 255, 255)), 231);
        let theme = Theme::new(ThemeName::TokyoNight, ColorMode::Indexed, false, false);
        assert!(matches!(theme.accent, Color::Indexed(_)));
    }

    #[test]
    fn highlighting_preserves_text() {
        let theme = Theme::new(ThemeName::TokyoNight, ColorMode::Plain, false, false);
        for line in [
            "let value = \"hello\"; // note",
            "def run(x): # comment",
            "const x = 42;",
            "  plain text",
        ] {
            let spans = highlight_text_line(line, syntax_for("main.rs"), &theme);
            let text: String = spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>()
                .join("");
            assert_eq!(text, line);
        }
    }

    #[test]
    fn modes_are_rendered() {
        let mut entry = Entry {
            name: "file".to_string(),
            path: hwe_vfs::VPath::new("/tmp/file"),
            kind: EntryKind::File,
            size: Some(1),
            modified: None,
            is_hidden: false,
            is_symlink: false,
            is_executable: false,
            mode: Some(0o100644),
        };
        assert_eq!(format_mode(&entry), "-rw-r--r--");

        entry.kind = EntryKind::Dir;
        entry.is_executable = true;
        entry.mode = Some(0o040755);
        assert_eq!(format_mode(&entry), "drwxr-xr-x");
    }

    #[test]
    fn extensions_are_normalized() {
        assert_eq!(extension("main.RS").as_deref(), Some("rs"));
        assert_eq!(extension("archive.tar.gz").as_deref(), Some("gz"));
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("README"), None);
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
