mod app;
mod model;
mod open;
mod providers;
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
    match app.mode {
        Mode::Search => search_key(app, key),
        Mode::Preview => preview_key(app, key),
    }
}

fn search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.mode = Mode::Preview,
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

fn preview_key(app: &mut App, key: KeyEvent) {
    let after_g = std::mem::take(&mut app.pending_g);
    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Esc | KeyCode::Char('/') => app.mode = Mode::Search,
        KeyCode::Down => app.select_next(),
        KeyCode::Up => app.select_previous(),
        KeyCode::Char('j') => app.scroll += 1,
        KeyCode::Char('k') => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::Char('g') if after_g => app.scroll = 0,
        KeyCode::Char('g') => app.pending_g = true,
        // The preview clamps this to the last page.
        KeyCode::Char('G') => app.scroll = usize::MAX,
        KeyCode::Char('o') => app.open = app.current(),
        _ => {}
    }
}
