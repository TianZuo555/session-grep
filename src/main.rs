mod app;
mod herdr;
mod model;
mod open;
mod providers;
mod rex;
mod search;
mod ui;

use std::time::Duration;

use anyhow::{Result, bail};
use chrono::{Duration as Days, Local};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use app::{App, Mode};
use model::{Session, one_line};

#[derive(Parser)]
#[command(
    name = "sgrep",
    about = "Search and resume recent coding-agent sessions"
)]
struct Args {
    /// How many days back to scan.
    #[arg(long, default_value_t = 30)]
    days: i64,
    /// Print the sessions and exit.
    #[arg(long)]
    list: bool,
    /// Print the transcript of one session (id or id prefix) and exit.
    #[arg(long, value_name = "ID")]
    dump: Option<String>,
    /// Print the resume command of one session (id or id prefix) and exit.
    #[arg(long, value_name = "ID")]
    print_command: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let (sessions, errors) = providers::discover(Local::now() - Days::days(args.days));
    // The TUI shows the errors in its status line.
    if args.list || args.dump.is_some() || args.print_command.is_some() {
        for error in &errors {
            eprintln!("sgrep: {error}");
        }
    }

    if args.list {
        for s in &sessions {
            println!(
                "{:<7} {:<8} {:<36} {:<40} {}",
                s.agent.label(),
                ui::ago(s.modified),
                s.id,
                one_line(&s.cwd, 40),
                one_line(&s.title, 80),
            );
        }
        return Ok(());
    }
    if let Some(id) = args.dump {
        for message in providers::transcript(find(&sessions, &id)?)? {
            println!("── {} ──\n{}\n", message.role.label(), message.text);
        }
        return Ok(());
    }
    if let Some(id) = args.print_command {
        println!("{}", open::describe(find(&sessions, &id)?));
        return Ok(());
    }

    let mut app = App::new(sessions, errors);
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result?;

    if let Some(index) = app.open {
        let session = &app.sessions[index];
        if rex::inside() {
            if rex::open(session)? {
                return Ok(());
            }
        } else if herdr::inside() {
            let own_pane = std::env::var("HERDR_PANE_ID").unwrap_or_default();
            if let Some(target) = herdr::target(session, &herdr::panes()?, &own_pane) {
                return herdr::open(session, &target);
            }
        }
        eprintln!("{}", open::describe(session));
        return Err(open::exec(session));
    }
    Ok(())
}

fn find<'a>(sessions: &'a [Session], id: &str) -> Result<&'a Session> {
    let mut found = sessions.iter().filter(|s| s.id.starts_with(id));
    match (found.next(), found.next()) {
        (Some(session), None) => Ok(session),
        (None, _) => bail!("no session matches {id}"),
        (Some(_), Some(_)) => bail!("more than one session matches {id}"),
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit && app.open.is_none() {
        app.tick();
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            handle_key(app, key);
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
    {
        app.quit = true;
        return;
    }
    if key.code == KeyCode::Enter {
        app.open = app.current();
        return;
    }
    match app.mode {
        Mode::Search => search_key(app, key),
        Mode::Browse => browse_key(app, key),
    }
}

fn search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.mode = Mode::Browse,
        KeyCode::Up => app.select_previous(),
        KeyCode::Down => app.select_next(),
        KeyCode::Backspace => {
            let mut query = app.query.clone();
            query.pop();
            app.set_query(query);
        }
        KeyCode::Char(c)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            let query = format!("{}{c}", app.query);
            app.set_query(query);
        }
        _ => {}
    }
}

fn browse_key(app: &mut App, key: KeyEvent) {
    let after_g = std::mem::take(&mut app.pending_g);
    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('s' | 'S') => app.mode = Mode::Search,
        KeyCode::Down => app.select_next(),
        KeyCode::Up => app.select_previous(),
        KeyCode::Char('j') => app.scroll += 1,
        KeyCode::Char('k') => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::Char('g') if after_g => app.scroll = 0,
        KeyCode::Char('g') => app.pending_g = true,
        // The preview clamps this to the last page.
        KeyCode::Char('G') => app.scroll = usize::MAX,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Agent, Source};

    fn app() -> App {
        App::new(
            ["first", "second"]
                .into_iter()
                .map(|id| Session {
                    agent: Agent::Pi,
                    id: id.into(),
                    cwd: "/work/project".into(),
                    title: id.into(),
                    modified: Local::now(),
                    source: Source::File("/missing/session.jsonl".into()),
                })
                .collect(),
            Vec::new(),
        )
    }

    fn press(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn search_is_explicit_and_escape_preserves_the_filter() {
        let mut app = app();
        assert_eq!(app.mode, Mode::Browse);
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.mode, Mode::Search);
        for ch in "second".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.query, "second");
        assert_eq!(app.current(), Some(1));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Browse);
        assert_eq!(app.query, "second");
        assert_eq!(app.current(), Some(1));
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.mode, Mode::Search);
        assert_eq!(app.query, "second");
    }

    #[test]
    fn enter_opens_the_selection_in_both_modes() {
        for mode in [Mode::Browse, Mode::Search] {
            let mut app = app();
            app.mode = mode;
            press(&mut app, KeyCode::Down);
            press(&mut app, KeyCode::Enter);
            assert_eq!(app.open, Some(1));
        }
        let mut empty = App::new(Vec::new(), Vec::new());
        press(&mut empty, KeyCode::Enter);
        assert_eq!(empty.open, None);
    }

    #[test]
    fn search_input_does_not_trigger_browse_shortcuts() {
        let mut app = app();
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(app.open, None);
        press(&mut app, KeyCode::Char('s'));
        for ch in "oqjks".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert_eq!(app.query, "oqjks");
        assert!(!app.quit);
        assert_eq!(app.open, None);
        assert_eq!(app.scroll, 0);
    }
}
