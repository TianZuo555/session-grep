use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use regex::Regex;
use tui_markdown::{Options, StyleSheet};

use crate::app::{App, Mode};
use crate::model::{Agent, Message, Role, one_line};
use crate::search::Query;
use crate::theme::{Colors, Theme};

/// The wrapped preview of one session for one query and one width.
/// Built once, then each frame draws only the visible rows.
pub struct Rendered {
    key: (usize, String, u16, Theme),
    lines: Vec<Line<'static>>,
    /// User-message lines reserve a separate two-column gutter.
    user_lines: Vec<bool>,
    /// The first wrapped row of each line.
    starts: Vec<usize>,
    rows: usize,
    first_match: Option<usize>,
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let colors = app.theme.colors();
    frame.render_widget(
        Block::new().style(Style::new().bg(colors.base).fg(colors.text)),
        frame.area(),
    );
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(main);
    draw_list(frame, app, left);
    draw_preview(frame, app, right);
    draw_status(frame, app, status);
    if app.mode == Mode::Search {
        let query = search_area(main);
        frame.render_widget(Clear, query);
        draw_query(frame, app, query);
    }
    if app.theme_menu.is_some() {
        draw_theme_menu(frame, app, frame.area());
    }
}

fn draw_theme_menu(frame: &mut Frame, app: &App, area: Rect) {
    let colors = app.theme.colors();
    let width = area.width.min(28);
    let height = area
        .height
        .saturating_sub(1)
        .min(Theme::ALL.len() as u16 + 2);
    let menu = Rect::new(
        area.x + 9.min(area.width.saturating_sub(width)),
        area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    );
    let items = Theme::ALL.map(|theme| {
        let marker = if theme == app.theme { "●" } else { " " };
        ListItem::new(format!("{marker} {}", theme.label()))
    });
    let list = List::new(items)
        .style(Style::new().bg(colors.base).fg(colors.text))
        .block(
            Block::bordered()
                .border_style(Style::new().fg(colors.mauve))
                .title(" theme "),
        )
        .highlight_symbol("› ")
        .highlight_style(
            Style::new()
                .bg(colors.surface0)
                .fg(colors.mauve)
                .add_modifier(Modifier::BOLD),
        );
    let selected = Theme::ALL
        .iter()
        .position(|theme| Some(*theme) == app.theme_menu);
    let mut state = ListState::default().with_selected(selected);
    frame.render_widget(Clear, menu);
    frame.render_stateful_widget(list, menu, &mut state);
}

/// Center the input on the upper golden-section point (38.2% of the height).
fn search_area(area: Rect) -> Rect {
    let width = area.width.min(76);
    let height = area.height.min(3);
    let center = ((u32::from(area.height) * 382 + 500) / 1000) as u16;
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + center.saturating_sub(height / 2).min(area.height - height),
        width,
        height,
    )
}

fn draw_query(frame: &mut Frame, app: &App, area: Rect) {
    let colors = app.theme.colors();
    let title = if app.scanning {
        " search · scanning content… "
    } else {
        " search "
    };
    let style = Style::new().fg(colors.mauve);
    let line = Line::from(vec![
        Span::styled("> ", style),
        Span::raw(app.query.clone()),
        Span::styled("▏", style),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(title);
    let overflow = line
        .width()
        .saturating_sub(area.width.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::new().bg(colors.base).fg(colors.text))
            .scroll((0, overflow.min(u16::MAX as usize) as u16))
            .block(block),
        area,
    );
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let colors = app.theme.colors();
    let width = area.width.saturating_sub(4) as usize;
    let query = Query::new(&app.query);
    let pattern = query.highlight_pattern();
    let current = app.current();
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&index| {
            let session = &app.sessions[index];
            let agent_style = Style::new()
                .fg(agent_color(session.agent, colors))
                .add_modifier(Modifier::BOLD);
            let agent_style = if query.agent() == Some(session.agent) {
                agent_style.fg(colors.crust).bg(colors.yellow)
            } else {
                agent_style
            };
            let mut head = highlighted_line(
                format!("{:<7}", session.agent.label()),
                agent_style,
                pattern.as_ref(),
                colors,
            );
            head.spans.push(Span::styled(
                format!("{:<8}", ago(session.modified)),
                Style::new().fg(colors.overlay1),
            ));
            head.spans.extend(
                highlighted_line(
                    one_line(&short_path(&session.cwd), width.saturating_sub(15)),
                    Style::new().fg(colors.teal),
                    pattern.as_ref(),
                    colors,
                )
                .spans,
            );
            let title = if session.title.is_empty() {
                "(no prompt)"
            } else {
                &session.title
            };
            let title = highlighted_line(
                format!("  {}", one_line(title, width.saturating_sub(2))),
                Style::new().fg(colors.subtext0),
                pattern.as_ref(),
                colors,
            );
            let mut lines = vec![head];
            if !session.names.is_empty() {
                lines.push(highlighted_line(
                    format!(
                        "  {}",
                        one_line(&session.names.join(" · "), width.saturating_sub(2))
                    ),
                    Style::new()
                        .fg(colors.lavender)
                        .add_modifier(Modifier::BOLD),
                    pattern.as_ref(),
                    colors,
                ));
            }
            lines.push(title);
            // Set the row background before drawing text so that match spans
            // retain their own background on the selected item.
            let style = if current == Some(index) {
                Style::new()
                    .bg(colors.surface0)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            ListItem::new(lines).style(style)
        })
        .collect();
    let title = format!(" sessions {}/{} ", app.visible.len(), app.sessions.len());
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(focus_border(app.mode == Mode::Browse, colors))
            .title(title),
    );
    let mut state = ListState::default().with_selected(app.current().map(|_| app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn highlighted_line(
    text: String,
    style: Style,
    pattern: Option<&Regex>,
    colors: Colors,
) -> Line<'static> {
    let mut line = Line::from(Span::styled(text, style));
    if let Some(pattern) = pattern {
        highlight(&mut line, pattern, colors);
    }
    line
}

fn draw_preview(frame: &mut Frame, app: &mut App, area: Rect) {
    let colors = app.theme.colors();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focus_border(app.mode == Mode::Browse, colors));
    let inner = block.inner(area);
    let Some(index) = app.current() else {
        frame.render_widget(Paragraph::new("no session").block(block), area);
        return;
    };
    let key = (
        index,
        app.query.trim().to_lowercase(),
        inner.width,
        app.theme,
    );
    if app.preview.as_ref().is_none_or(|r| r.key != key) {
        let messages = app.transcripts.get(index);
        app.preview = Some(render(key, messages, inner.width));
    }
    let preview = app.preview.as_ref().expect("built above");

    let height = inner.height as usize;
    if std::mem::take(&mut app.jump_to_match) {
        app.scroll = preview.first_match.map_or(0, |row| row.saturating_sub(2));
    }
    app.scroll = app.scroll.min(preview.rows.saturating_sub(height));

    // Draw only the lines that reach the viewport.
    let first = preview
        .starts
        .partition_point(|&start| start <= app.scroll)
        .saturating_sub(1);
    let last = preview
        .starts
        .partition_point(|&start| start < app.scroll + height);
    let title = format!(
        " preview {}-{}/{} ",
        app.scroll + 1,
        (app.scroll + height).min(preview.rows),
        preview.rows
    );
    frame.render_widget(block.title(title), area);
    draw_preview_rows(frame, preview, inner, app.scroll, first..last, colors);
}

fn draw_preview_rows(
    frame: &mut Frame,
    preview: &Rendered,
    inner: Rect,
    scroll: usize,
    visible: std::ops::Range<usize>,
    colors: Colors,
) {
    for index in visible {
        let start = preview.starts[index];
        let end = preview
            .starts
            .get(index + 1)
            .copied()
            .unwrap_or(preview.rows);
        let top = start.max(scroll);
        let bottom = end.min(scroll + inner.height as usize);
        if top >= bottom {
            continue;
        }
        let mut area = Rect::new(
            inner.x,
            inner.y + (top - scroll) as u16,
            inner.width,
            (bottom - top) as u16,
        );
        if preview.user_lines[index] {
            let bars = vec![Line::styled("▌", colors.blue); area.height as usize];
            frame.render_widget(
                Paragraph::new(bars),
                Rect::new(area.x, area.y, area.width.min(1), area.height),
            );
            let gutter = area.width.min(2);
            area.x += gutter;
            area.width -= gutter;
        }
        frame.render_widget(
            Paragraph::new(preview.lines[index].clone())
                .wrap(Wrap { trim: false })
                .scroll(((top - start).min(u16::MAX as usize) as u16, 0)),
            area,
        );
    }
}

fn render(
    key: (usize, String, u16, Theme),
    messages: &Result<Vec<Message>, String>,
    width: u16,
) -> Rendered {
    let colors = key.3.colors();
    let pattern = Query::new(&key.1).highlight_pattern();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut user_lines = Vec::new();
    match messages {
        Err(error) => lines.push(Line::styled(
            format!("cannot read session: {error}"),
            colors.red,
        )),
        Ok(messages) if messages.is_empty() => {
            lines.push(Line::styled("no conversation", colors.overlay0))
        }
        Ok(messages) => {
            let options = Options::new(Markdown(colors)).table_width(width);
            for message in messages {
                match message.role {
                    // The gutter is drawn separately on every wrapped row.
                    Role::User => lines.extend(
                        message
                            .text
                            .lines()
                            .map(|raw| Line::styled(raw.to_string(), colors.blue)),
                    ),
                    _ => lines.extend(
                        tui_markdown::from_str_with_options(&message.text, &options)
                            .lines
                            .into_iter()
                            .map(owned),
                    ),
                }
                user_lines.resize(lines.len(), message.role == Role::User);
                lines.push(Line::default());
                user_lines.push(false);
            }
        }
    }
    user_lines.resize(lines.len(), false);
    let mut first_match_line = None;
    if let Some(pattern) = &pattern {
        for (index, line) in lines.iter_mut().enumerate() {
            if highlight(line, pattern, colors) && first_match_line.is_none() {
                first_match_line = Some(index);
            }
        }
    }
    let mut starts = Vec::with_capacity(lines.len());
    let mut rows = 0;
    for (index, line) in lines.iter().enumerate() {
        starts.push(rows);
        let content_width = width.saturating_sub(if user_lines[index] { 2 } else { 0 });
        rows += Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(content_width)
            .max(1);
    }
    Rendered {
        first_match: first_match_line.map(|line| starts[line]),
        key,
        lines,
        user_lines,
        starts,
        rows,
    }
}

/// Render markdown in the selected theme, without the `#` heading markers and
/// the code fences.
#[derive(Clone)]
struct Markdown(Colors);

impl StyleSheet for Markdown {
    fn heading(&self, level: u8) -> Style {
        let color = match level {
            1 => self.0.mauve,
            2 => self.0.lavender,
            _ => self.0.blue,
        };
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    }

    fn heading_marker(&self, _level: u8) -> &str {
        ""
    }

    fn code(&self) -> Style {
        Style::new().fg(self.0.green).bg(self.0.mantle)
    }

    fn code_block_fence(&self) -> &str {
        ""
    }

    fn link(&self) -> Style {
        Style::new()
            .fg(self.0.sapphire)
            .add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        Style::new()
            .fg(self.0.overlay1)
            .add_modifier(Modifier::ITALIC)
    }

    fn html(&self) -> Style {
        Style::new().fg(self.0.overlay1)
    }

    fn table_header(&self) -> Style {
        Style::new()
            .fg(self.0.lavender)
            .add_modifier(Modifier::BOLD)
    }

    fn table_border(&self) -> Style {
        Style::new().fg(self.0.surface2)
    }

    fn list_marker(&self) -> Style {
        Style::new().fg(self.0.peach)
    }
}

/// The rendered markdown borrows from the message text. The cache keeps the
/// lines longer than that, so they must own their text.
fn owned(line: Line<'_>) -> Line<'static> {
    let spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect();
    Line::from(spans).style(line.style)
}

/// Split the spans of a line so that each match shows on a yellow background.
/// A match that crosses two spans is not marked. Returns true when the line
/// holds a marked match.
fn highlight(line: &mut Line<'static>, pattern: &Regex, colors: Colors) -> bool {
    let mut found_any = false;
    let mut spans = Vec::with_capacity(line.spans.len());
    for span in line.spans.drain(..) {
        let text = span.content.as_ref();
        let mut end = 0;
        for found in pattern.find_iter(text) {
            found_any = true;
            spans.push(Span::styled(
                text[end..found.start()].to_string(),
                span.style,
            ));
            spans.push(Span::styled(
                found.as_str().to_string(),
                span.style.fg(colors.crust).bg(colors.yellow),
            ));
            end = found.end();
        }
        if end == 0 {
            spans.push(span);
        } else {
            spans.push(Span::styled(text[end..].to_string(), span.style));
        }
    }
    line.spans = spans;
    found_any
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let colors = app.theme.colors();
    let (mode, color, hint) = match app.mode {
        Mode::Search => (
            " SEARCH ",
            colors.mauve,
            "type to filter  ↑↓ select  Enter open  Esc close search  ^D quit",
        ),
        Mode::Browse => (
            " BROWSE ",
            colors.green,
            "s search  T theme  Enter open  ↑↓ select  j/k scroll  q quit",
        ),
    };
    let hint = if app.theme_menu.is_some() {
        "↑↓ select theme  Enter apply  Esc cancel"
    } else {
        hint
    };
    let mut spans = vec![
        Span::styled(
            mode,
            Style::new()
                .bg(color)
                .fg(colors.badge_text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} · {hint} ", app.theme.label()),
            Style::new().fg(colors.overlay1),
        ),
    ];
    if app.mode == Mode::Browse && !app.query.is_empty() {
        spans.push(Span::styled(
            format!(" filter: {}", app.query),
            Style::new().fg(colors.mauve),
        ));
    }
    if let Some(first) = app.errors.first() {
        let more = match app.errors.len() {
            1 => String::new(),
            n => format!(" (+{} more, see sgrep --list)", n - 1),
        };
        spans.push(Span::styled(
            format!("  ⚠ {first}{more}"),
            Style::new().fg(colors.red),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(colors.mantle)),
        area,
    );
}

/// The border of the panel that has the keyboard focus is brighter.
fn focus_border(focused: bool, colors: Colors) -> Style {
    Style::new().fg(if focused {
        colors.mauve
    } else {
        colors.surface2
    })
}

fn agent_color(agent: Agent, colors: Colors) -> Color {
    match agent {
        Agent::Pi => colors.mauve,
        Agent::Codex => colors.green,
        Agent::Claude => colors.peach,
        Agent::Cursor => colors.blue,
        Agent::Agy => colors.sapphire,
        Agent::Devin => colors.yellow,
    }
}

fn short_path(path: &str) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home.to_str()?).map(String::from)) {
        Some(rest) => format!("~{rest}"),
        None => path.to_string(),
    }
}

pub fn ago(time: DateTime<Local>) -> String {
    match (Local::now() - time).num_seconds().max(0) {
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::pattern;

    fn list_app() -> App {
        use crate::model::{Session, Source};

        App::new(
            (0..2)
                .map(|index| Session {
                    agent: Agent::Devin,
                    id: index.to_string(),
                    cwd: "/work/login".into(),
                    title: "Fix LOGIN and test".into(),
                    names: vec!["Login pane".into()],
                    pane_name: None,
                    tab_name: None,
                    modified: Local::now(),
                    source: Source::Row {
                        db: "/missing/session.db".into(),
                        session_id: index.to_string(),
                    },
                })
                .collect(),
            Vec::new(),
        )
    }

    fn list_buffer(app: &App) -> ratatui::buffer::Buffer {
        use ratatui::{Terminal, backend::TestBackend};

        let mut terminal = Terminal::new(TestBackend::new(90, 12)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Block::new().style(Style::new().bg(app.theme.colors().base)),
                    frame.area(),
                );
                draw_list(frame, app, frame.area());
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn list_highlights_agent_names_paths_and_titles_even_on_the_selected_item() {
        let mut app = list_app();
        app.set_query("devin + LOGIN".into());
        for theme in Theme::ALL {
            app.theme = theme;
            let colors = theme.colors();
            for mode in [Mode::Search, Mode::Browse] {
                app.mode = mode;
                let buffer = list_buffer(&app);
                for (head, names, title) in [(1, 2, 3), (4, 5, 6)] {
                    for (start, end, row) in
                        [(1, 6, head), (22, 27, head), (3, 8, names), (7, 12, title)]
                    {
                        for column in start..end {
                            assert_eq!(
                                buffer[(column, row)].bg,
                                colors.yellow,
                                "{theme:?} at {column},{row}"
                            );
                            assert_eq!(buffer[(column, row)].fg, colors.crust);
                        }
                    }
                    assert_ne!(buffer[(8, head)].bg, colors.yellow);
                }
                assert_eq!(buffer[(80, 3)].bg, colors.surface0);
                assert_eq!(buffer[(80, 6)].bg, colors.base);
            }
        }
    }

    #[test]
    fn clearing_search_removes_list_match_highlights() {
        let mut app = list_app();
        app.set_query("devin login".into());
        app.set_query(String::new());
        let buffer = list_buffer(&app);
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.bg != app.theme.colors().yellow)
        );
        assert_eq!(buffer[(80, 3)].bg, app.theme.colors().surface0);
    }

    #[test]
    fn content_only_matches_do_not_highlight_unmatched_list_text() {
        let mut app = list_app();
        app.set_query("body-only".into());
        // A content scan can add a session whose displayed metadata does not match.
        app.visible = vec![0];
        let buffer = list_buffer(&app);
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.bg != app.theme.colors().yellow)
        );
    }

    fn preview_buffer(
        preview: &Rendered,
        width: u16,
        height: u16,
        scroll: usize,
    ) -> ratatui::buffer::Buffer {
        use ratatui::{Terminal, backend::TestBackend};

        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                draw_preview_rows(
                    frame,
                    preview,
                    frame.area(),
                    scroll,
                    0..preview.lines.len(),
                    preview.key.3.colors(),
                );
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn user_bar_covers_every_soft_wrapped_row() {
        let messages = Ok(vec![
            Message::new(Role::User, "abcdefghijklmnopqrstuvwx"),
            Message::new(Role::Assistant, "done"),
        ]);
        for (width, rows) in [(8, 4), (12, 3), (80, 1)] {
            let preview = render((0, String::new(), width, Theme::Frappe), &messages, width);
            assert_eq!(preview.starts[1], rows);
            let buffer = preview_buffer(&preview, width, 10, 0);
            for row in 0..rows as u16 {
                assert_eq!(buffer[(0, row)].symbol(), "▌");
                assert_eq!(buffer[(1, row)].symbol(), " ");
            }
            assert_eq!(buffer[(0, rows as u16)].symbol(), " ");
            assert_eq!(buffer[(0, rows as u16 + 1)].symbol(), "d");
        }
    }

    #[test]
    fn user_bar_stays_visible_when_scrolling_into_a_wrapped_message() {
        let messages = Ok(vec![
            Message::new(Role::User, "abcdefghijklmnopqrstuvwx"),
            Message::new(Role::Assistant, "done"),
        ]);
        let preview = render((0, String::new(), 8, Theme::Frappe), &messages, 8);
        let buffer = preview_buffer(&preview, 8, 6, 1);
        for row in 0..3 {
            assert_eq!(buffer[(0, row)].symbol(), "▌");
        }
        assert_eq!(buffer[(2, 0)].symbol(), "g");
        assert_eq!(buffer[(0, 3)].symbol(), " ");
        assert_eq!(buffer[(0, 4)].symbol(), "d");
    }

    #[test]
    fn user_bar_covers_chinese_and_blank_lines_without_breaking_highlights() {
        let messages = Ok(vec![
            Message::new(Role::User, "中文中文\n\n终点"),
            Message::new(Role::Assistant, "done"),
        ]);
        for theme in Theme::ALL {
            let preview = render((0, "终点".into(), 6, theme), &messages, 6);
            let buffer = preview_buffer(&preview, 6, 7, 0);
            for row in 0..4 {
                assert_eq!(buffer[(0, row)].symbol(), "▌");
                assert_eq!(buffer[(0, row)].fg, theme.colors().blue);
            }
            assert_eq!(buffer[(2, 0)].symbol(), "中");
            assert_eq!(buffer[(2, 1)].symbol(), "中");
            assert_eq!(buffer[(2, 2)].symbol(), " ");
            assert_eq!(buffer[(2, 3)].symbol(), "终");
            assert_eq!(buffer[(2, 3)].bg, theme.colors().yellow);
            assert_eq!(preview.first_match, Some(3));
            assert_eq!(buffer[(0, 4)].symbol(), " ");
            assert_eq!(buffer[(0, 5)].symbol(), "d");
        }
    }

    #[test]
    fn theme_menu_shows_choices_and_tracks_selection() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = App::new(Vec::new(), Vec::new());
        app.theme_menu = Some(Theme::Midnight);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("┌ theme "));
        assert!(screen.contains("● Frappe"));
        assert!(screen.contains("›   Midnight"));
        for theme in Theme::ALL {
            assert!(screen.contains(theme.label()));
        }
        assert!(screen.contains("Enter apply"));
        assert!(screen.contains("Esc cancel"));
        app.theme_menu = None;
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!screen.contains("┌ theme "));
    }

    #[test]
    fn theme_menu_fits_small_terminals() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = App::new(Vec::new(), Vec::new());
        app.theme_menu = Some(Theme::Akua);
        for (width, height) in [(1, 1), (12, 4), (28, 8)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        }
    }

    #[test]
    fn search_uses_the_upper_golden_section_and_stays_in_bounds() {
        assert_eq!(
            search_area(Rect::new(0, 0, 120, 50)),
            Rect::new(22, 18, 76, 3)
        );
        for width in [0, 1, 20, 76, 120] {
            for height in [0, 1, 2, 3, 8, 24, 50, 80] {
                let area = Rect::new(4, 7, width, height);
                let query = search_area(area);
                assert!(query.x >= area.x && query.right() <= area.right());
                assert!(query.y >= area.y && query.bottom() <= area.bottom());
                if height >= 8 {
                    let expected = f64::from(height) * 0.382;
                    let center = f64::from(query.y - area.y + query.height / 2);
                    assert!((center - expected).abs() <= 0.5);
                }
            }
        }
    }

    #[test]
    fn changing_theme_rebuilds_cached_preview_and_repaints_the_ui() {
        use crate::model::{Session, Source};
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = App::new(
            vec![Session {
                agent: Agent::Pi,
                id: "test".into(),
                cwd: "/work/project".into(),
                title: "Theme test".into(),
                names: vec!["Login pane".into(), "Review tab".into()],
                pane_name: None,
                tab_name: None,
                modified: Local::now(),
                source: Source::File("/missing/session.jsonl".into()),
            }],
            Vec::new(),
        );
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        for theme in [
            Theme::Frappe,
            Theme::Paper,
            Theme::Midnight,
            Theme::Orbit,
            Theme::Akua,
        ] {
            app.theme = theme;
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            assert_eq!(app.preview.as_ref().unwrap().key.3, theme);
            assert_eq!(
                app.preview.as_ref().unwrap().lines[0].style.fg,
                Some(theme.colors().red)
            );
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(1, 38)].bg, theme.colors().base);
            let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(screen.contains(theme.label()));
            assert!(screen.contains("Login pane · Review tab"));
            assert!(screen.contains("Theme test"));
        }
    }

    #[test]
    fn every_theme_colors_markdown_and_compound_search_matches() {
        let messages = Ok(vec![
            Message::new(Role::User, "Find a sentence"),
            Message::new(Role::Assistant, "# Heading\n\nThe sentence is here."),
        ]);
        for theme in [
            Theme::Frappe,
            Theme::Paper,
            Theme::Midnight,
            Theme::Orbit,
            Theme::Akua,
        ] {
            let rendered = render((0, "devin + sentence".into(), 80, theme), &messages, 80);
            let colors = theme.colors();
            assert!(rendered.first_match.is_some());
            assert!(
                rendered
                    .lines
                    .iter()
                    .flat_map(|line| &line.spans)
                    .any(|span| {
                        span.content == "sentence" && span.style.bg == Some(colors.yellow)
                    })
            );
            assert!(rendered.lines.iter().any(|line| {
                line.to_string() == "Heading"
                    && line
                        .spans
                        .iter()
                        .any(|span| span.style.fg.or(line.style.fg) == Some(colors.mauve))
            }));
        }
    }

    #[test]
    fn search_overlay_is_only_visible_in_search_mode() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = App::new(Vec::new(), Vec::new());
        app.set_query("needle".into());
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        for mode in [Mode::Browse, Mode::Search, Mode::Browse] {
            app.mode = mode;
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert_eq!(screen.contains("┌ search "), mode == Mode::Search);
            assert_eq!(screen.contains("> needle▏"), mode == Mode::Search);
            assert!(screen.contains("sessions 0/0"));
            assert!(screen.contains("Enter open"));
        }
    }

    #[test]
    fn highlight_marks_each_match() {
        let mut line = Line::from("a Foo b foo");
        assert!(highlight(
            &mut line,
            &pattern("foo"),
            Theme::default().colors()
        ));
        let texts: Vec<_> = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, ["a ", "Foo", " b ", "foo", ""]);
    }

    #[test]
    fn render_hides_roles_and_markdown_markers() {
        let messages = Ok(vec![
            Message::new(Role::User, "one\ntwo"),
            Message::new(Role::Assistant, "## Title\n\n```rust\nlet needle = 1;\n```"),
        ]);
        let rendered = render((0, "needle".into(), 80, Theme::default()), &messages, 80);
        let text: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();
        assert_eq!(text[..3], ["one", "two", ""]);
        assert_eq!(rendered.user_lines[..3], [true, true, false]);
        assert!(!text.iter().any(|l| l.contains("##") || l.contains("```")));
        assert!(text.iter().any(|l| l == "Title"));
        let needle = text.iter().position(|l| l.contains("let needle")).unwrap();
        assert_eq!(rendered.first_match, Some(rendered.starts[needle]));
    }
}
