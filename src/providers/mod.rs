mod agy;
mod claude;
mod codex;
mod cursor;
mod devin;
mod json;
mod jsonl;
mod pi;
mod protobuf;

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rayon::prelude::*;
use rusqlite::{Connection, OpenFlags};

use crate::model::{Agent, Message, Session, Source};

/// The outer error stops a whole provider. An inner error skips one session.
type Discover = fn(DateTime<Local>) -> Result<Vec<Result<Session>>>;

const PROVIDERS: [(Agent, Discover); 6] = [
    (Agent::Pi, pi::discover),
    (Agent::Codex, codex::discover),
    (Agent::Claude, claude::discover),
    (Agent::Cursor, cursor::discover),
    (Agent::Agy, agy::discover),
    (Agent::Devin, devin::discover),
];

/// Every session of every agent that changed at or after `since`, newest
/// first, and one error line for each agent that failed.
pub fn discover(since: DateTime<Local>) -> (Vec<Session>, Vec<String>) {
    let results: Vec<_> = PROVIDERS
        .par_iter()
        .map(|(agent, discover)| (agent, discover(since)))
        .collect();
    let mut sessions = Vec::new();
    let mut errors = Vec::new();
    for (agent, result) in results {
        let items = result.unwrap_or_else(|error| vec![Err(error)]);
        for item in items {
            match item {
                Ok(session) => sessions.push(session),
                Err(error) => errors.push(format!("{}: {error:#}", agent.label())),
            }
        }
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    (sessions, errors)
}

pub fn transcript(session: &Session) -> Result<Vec<Message>> {
    match (session.agent, &session.source) {
        (Agent::Pi, Source::File(path)) => pi::transcript(path),
        (Agent::Codex, Source::File(path)) => codex::transcript(path),
        (Agent::Claude, Source::File(path)) => claude::transcript(path),
        (Agent::Cursor, Source::Database(db)) => cursor::transcript(db),
        (Agent::Agy, Source::Database(db)) => agy::transcript(db),
        (Agent::Devin, Source::Row { db, session_id }) => devin::transcript(db, session_id),
        (agent, source) => unreachable!("{agent:?} never builds {source:?}"),
    }
}

fn open_readonly(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("cannot open {}", path.display()))
}
