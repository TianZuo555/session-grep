use catppuccin::{FlavorColors, PALETTE};
use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use regex::Regex;
use tui_markdown::{Options, StyleSheet};

use crate::app::{App, Mode};
use crate::model::{Agent, Message, Role, one_line};
use crate::search::pattern;

/// The Catppuccin Frappé palette.
const C: FlavorColors = PALETTE.frappe.colors;

const fn c(color: catppuccin::Color) -> Color {
    Color::Rgb(color.rgb.r, color.rgb.g, color.rgb.b)
}

/// The wrapped preview of one session for one query and one width.
/// Built once, then each frame draws only the visible rows.
pub struct Rendered {
    key: (usize, String, u16),
    lines: Vec<Line<'static>>,
    /// The first wrapped row of each line.
    starts: Vec<usize>,
    rows: usize,
    first_match: Option<usize>,
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    frame.render_widget(
        Block::new().style(Style::new().bg(c(C.base)).fg(c(C.text))),
        frame.area(),
    );
    let [main, status] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(main);
    let [query, list] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(left);
    draw_query(frame, app, query);
    draw_list(frame, app, list);
    draw_preview(frame, app, right);
    draw_status(frame, app, status);
}

fn draw_query(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app.scanning {
        " search · scanning content… "
    } else {
        " search "
    };
    let style = match app.mode {
        Mode::Search => Style::new().fg(c(C.mauve)),
        Mode::Preview => Style::new().fg(c(C.surface2)),
    };
    let caret = if app.mode == Mode::Search { "▏" } else { "" };
    let line = Line::from(vec![
        Span::styled("> ", style),
        Span::raw(app.query.clone()),
        Span::styled(caret, style),
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(title);
    frame.render_widget(Paragraph::new(line).block(block), area);
}

fn draw_list(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&index| {
            let session = &app.sessions[index];
            let head = Line::from(vec![
                Span::styled(
                    format!("{:<7}", session.agent.label()),
                    Style::new()
                        .fg(agent_color(session.agent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<8}", ago(session.modified)),
                    Style::new().fg(c(C.overlay1)),
                ),
                Span::styled(
                    one_line(&short_path(&session.cwd), width.saturating_sub(15)),
                    Style::new().fg(c(C.teal)),
                ),
            ]);
            let title = if session.title.is_empty() {
                "(no prompt)"
            } else {
                &session.title
            };
            let title = Line::from(Span::styled(
                format!("  {}", one_line(title, width.saturating_sub(2))),
                Style::new().fg(c(C.subtext0)),
            ));
            ListItem::new(vec![head, title])
        })
        .collect();
    let title = format!(" sessions {}/{} ", app.visible.len(), app.sessions.len());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(focus_border(app.mode == Mode::Preview))
                .title(title),
        )
        .highlight_style(Style::new().bg(c(C.surface0)).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(app.current().map(|_| app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(focus_border(app.mode == Mode::Preview));
    let inner = block.inner(area);
    let Some(index) = app.current() else {
        frame.render_widget(Paragraph::new("no session").block(block), area);
        return;
    };
    let key = (index, app.query.trim().to_lowercase(), inner.width);
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
    let offset = app.scroll - preview.starts.get(first).copied().unwrap_or(0);
    let title = format!(
        " preview {}-{}/{} ",
        app.scroll + 1,
        (app.scroll + height).min(preview.rows),
        preview.rows
    );
    let paragraph = Paragraph::new(preview.lines[first..last].to_vec())
        .wrap(Wrap { trim: false })
        .scroll((offset as u16, 0))
        .block(block.title(title));
    frame.render_widget(paragraph, area);
}

fn render(
    key: (usize, String, u16),
    messages: &Result<Vec<Message>, String>,
    width: u16,
) -> Rendered {
    let pattern = (!key.1.is_empty()).then(|| pattern(&key.1));
    let mut lines: Vec<Line<'static>> = Vec::new();
    match messages {
        Err(error) => lines.push(Line::styled(
            format!("cannot read session: {error}"),
            c(C.red),
        )),
        Ok(messages) if messages.is_empty() => {
            lines.push(Line::styled("no conversation", c(C.overlay0)))
        }
        Ok(messages) => {
            let options = Options::new(Markdown).table_width(width);
            for message in messages {
                match message.role {
                    // A bar marks the user turns, so that no role label is necessary.
                    Role::User => lines.extend(message.text.lines().map(|raw| {
                        Line::from(vec![
                            Span::styled("▌ ", c(C.blue)),
                            Span::styled(raw.to_string(), c(C.blue)),
                        ])
                    })),
                    _ => lines.extend(
                        tui_markdown::from_str_with_options(&message.text, &options)
                            .lines
                            .into_iter()
                            .map(owned),
                    ),
                }
                lines.push(Line::default());
            }
        }
    }
    let mut first_match_line = None;
    if let Some(pattern) = &pattern {
        for (index, line) in lines.iter_mut().enumerate() {
            if highlight(line, pattern) && first_match_line.is_none() {
                first_match_line = Some(index);
            }
        }
    }
    let mut starts = Vec::with_capacity(lines.len());
    let mut rows = 0;
    for line in &lines {
        starts.push(rows);
        rows += Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(width)
            .max(1);
    }
    Rendered {
        first_match: first_match_line.map(|line| starts[line]),
        key,
        lines,
        starts,
        rows,
    }
}

/// Render the markdown in Frappé colors, without the `#` heading markers and
/// the code fences.
#[derive(Clone)]
struct Markdown;

impl StyleSheet for Markdown {
    fn heading(&self, level: u8) -> Style {
        let color = match level {
            1 => c(C.mauve),
            2 => c(C.lavender),
            _ => c(C.blue),
        };
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    }

    fn heading_marker(&self, _level: u8) -> &str {
        ""
    }

    fn code(&self) -> Style {
        Style::new().fg(c(C.green)).bg(c(C.mantle))
    }

    fn code_block_fence(&self) -> &str {
        ""
    }

    fn link(&self) -> Style {
        Style::new()
            .fg(c(C.sapphire))
            .add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        Style::new()
            .fg(c(C.overlay2))
            .add_modifier(Modifier::ITALIC)
    }

    fn html(&self) -> Style {
        Style::new().fg(c(C.overlay1))
    }

    fn table_header(&self) -> Style {
        Style::new().fg(c(C.lavender)).add_modifier(Modifier::BOLD)
    }

    fn table_border(&self) -> Style {
        Style::new().fg(c(C.surface2))
    }

    fn list_marker(&self) -> Style {
        Style::new().fg(c(C.peach))
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
fn highlight(line: &mut Line<'static>, pattern: &Regex) -> bool {
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
                span.style.fg(c(C.crust)).bg(c(C.yellow)),
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
    let (mode, color, hint) = match app.mode {
        Mode::Search => (
            " SEARCH ",
            c(C.mauve),
            "type to filter  ↑↓ select  Enter preview  ^D quit",
        ),
        Mode::Preview => (
            " PREVIEW ",
            c(C.green),
            "↑↓ select  j/k scroll  gg/G top/end  o open  Esc search  q quit",
        ),
    };
    let mut spans = vec![
        Span::styled(
            mode,
            Style::new()
                .bg(color)
                .fg(c(C.crust))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {hint} "), Style::new().fg(c(C.overlay1))),
    ];
    if let Some(first) = app.errors.first() {
        let more = match app.errors.len() {
            1 => String::new(),
            n => format!(" (+{} more, see sgrep --list)", n - 1),
        };
        spans.push(Span::styled(
            format!("  ⚠ {first}{more}"),
            Style::new().fg(c(C.red)),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(c(C.mantle))),
        area,
    );
}

/// The border of the panel that has the keyboard focus is brighter.
fn focus_border(focused: bool) -> Style {
    Style::new().fg(if focused { c(C.mauve) } else { c(C.surface2) })
}

fn agent_color(agent: Agent) -> Color {
    match agent {
        Agent::Pi => c(C.mauve),
        Agent::Codex => c(C.green),
        Agent::Claude => c(C.peach),
        Agent::Cursor => c(C.blue),
        Agent::Agy => c(C.sapphire),
        Agent::Devin => c(C.yellow),
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

    #[test]
    fn highlight_marks_each_match() {
        let mut line = Line::from("a Foo b foo");
        assert!(highlight(&mut line, &pattern("foo")));
        let texts: Vec<_> = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, ["a ", "Foo", " b ", "foo", ""]);
    }

    #[test]
    fn render_hides_roles_and_markdown_markers() {
        let messages = Ok(vec![
            Message::new(Role::User, "one\ntwo"),
            Message::new(Role::Assistant, "## Title\n\n```rust\nlet needle = 1;\n```"),
        ]);
        let rendered = render((0, "needle".into(), 80), &messages, 80);
        let text: Vec<String> = rendered.lines.iter().map(|l| l.to_string()).collect();
        assert_eq!(text[..3], ["▌ one", "▌ two", ""]);
        assert!(!text.iter().any(|l| l.contains("##") || l.contains("```")));
        assert!(text.iter().any(|l| l == "Title"));
        let needle = text.iter().position(|l| l.contains("let needle")).unwrap();
        assert_eq!(rendered.first_match, Some(rendered.starts[needle]));
    }
}
