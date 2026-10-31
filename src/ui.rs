use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use regex::Regex;

use crate::app::{App, Mode};
use crate::model::{Agent, Message, Role, one_line};
use crate::search::pattern;

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
        Mode::Search => Style::default().fg(Color::Yellow),
        Mode::Normal => Style::default().fg(Color::DarkGray),
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
                    Style::default()
                        .fg(agent_color(session.agent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:<8}", ago(session.modified)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    one_line(&short_path(&session.cwd), width.saturating_sub(15)),
                    Style::default().fg(Color::Cyan),
                ),
            ]);
            let title = if session.title.is_empty() {
                "(no prompt)"
            } else {
                &session.title
            };
            let title = Line::from(Span::styled(
                format!("  {}", one_line(title, width.saturating_sub(2))),
                Style::default().fg(Color::Gray),
            ));
            ListItem::new(vec![head, title])
        })
        .collect();
    let title = format!(" sessions {}/{} ", app.visible.len(), app.sessions.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .bg(Color::Indexed(238))
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(app.current().map(|_| app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default().borders(Borders::ALL);
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
    let mut first_match_line = None;
    match messages {
        Err(error) => lines.push(Line::styled(
            format!("cannot read session: {error}"),
            Color::Red,
        )),
        Ok(messages) if messages.is_empty() => {
            lines.push(Line::styled("no transcript", Color::DarkGray))
        }
        Ok(messages) => {
            for message in messages.iter().filter(|m| m.role != Role::System) {
                lines.push(Line::styled(
                    format!("── {} ──", message.role.label()),
                    Style::default()
                        .fg(role_color(message.role))
                        .add_modifier(Modifier::BOLD),
                ));
                // A tool call or a tool result fills one line only.
                let text = match message.role {
                    Role::Tool => one_line(&message.text, width.saturating_sub(1) as usize),
                    _ => message.text.clone(),
                };
                for raw in text.lines() {
                    let line = highlight(raw, text_color(message.role), pattern.as_ref());
                    if first_match_line.is_none() && line.spans.len() > 1 {
                        first_match_line = Some(lines.len());
                    }
                    lines.push(line);
                }
                lines.push(Line::default());
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

/// Split one line into spans so that each match shows in reverse video.
fn highlight(raw: &str, color: Color, pattern: Option<&Regex>) -> Line<'static> {
    let base = Style::default().fg(color);
    let Some(pattern) = pattern else {
        return Line::styled(raw.to_string(), base);
    };
    let mut spans = Vec::new();
    let mut end = 0;
    for found in pattern.find_iter(raw) {
        spans.push(Span::styled(raw[end..found.start()].to_string(), base));
        spans.push(Span::styled(
            found.as_str().to_string(),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ));
        end = found.end();
    }
    if spans.is_empty() {
        return Line::styled(raw.to_string(), base);
    }
    spans.push(Span::styled(raw[end..].to_string(), base));
    Line::from(spans)
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let (mode, color, hint) = match app.mode {
        Mode::Search => (
            " SEARCH ",
            Color::Yellow,
            "type to filter  ↑↓ select  Esc/Enter normal",
        ),
        Mode::Normal => (
            " NORMAL ",
            Color::Blue,
            "j/k select  J/K scroll  gg/G top/end  / search  o open  q quit",
        ),
    };
    let mut spans = vec![
        Span::styled(
            mode,
            Style::default()
                .bg(color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {hint} "), Style::default().fg(Color::DarkGray)),
    ];
    if let Some(index) = app.current() {
        let command = app.sessions[index].resume_command().join(" ");
        spans.push(Span::styled(
            format!(" o → {command}"),
            Style::default().fg(Color::Green),
        ));
    }
    if let Some(first) = app.errors.first() {
        let more = match app.errors.len() {
            1 => String::new(),
            n => format!(" (+{} more, see sgrep --list)", n - 1),
        };
        spans.push(Span::styled(
            format!("  ⚠ {first}{more}"),
            Style::default().fg(Color::Red),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn agent_color(agent: Agent) -> Color {
    match agent {
        Agent::Pi => Color::Magenta,
        Agent::Codex => Color::Green,
        Agent::Claude => Color::LightRed,
        Agent::Cursor => Color::Blue,
        Agent::Agy => Color::LightBlue,
        Agent::Devin => Color::LightYellow,
    }
}

fn role_color(role: Role) -> Color {
    match role {
        Role::User => Color::Yellow,
        Role::Assistant => Color::Green,
        Role::Thinking => Color::Magenta,
        Role::Tool => Color::Blue,
        Role::System => Color::DarkGray,
    }
}

fn text_color(role: Role) -> Color {
    match role {
        Role::Thinking | Role::System => Color::DarkGray,
        Role::Tool => Color::Indexed(109),
        Role::User | Role::Assistant => Color::White,
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
        let line = highlight("a Foo b foo", Color::White, Some(&pattern("foo")));
        let texts: Vec<_> = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, ["a ", "Foo", " b ", "foo", ""]);
    }

    #[test]
    fn render_finds_first_match_row_and_skips_system() {
        let messages = Ok(vec![
            Message::new(Role::System, "needle in the system prompt"),
            Message::new(Role::User, "one\ntwo"),
            Message::new(Role::Assistant, "the needle"),
        ]);
        let rendered = render((0, "needle".into(), 80), &messages, 80);
        // user header, one, two, blank, assistant header, match
        assert_eq!(rendered.first_match, Some(5));
        assert_eq!(rendered.rows, 7);
    }
}
