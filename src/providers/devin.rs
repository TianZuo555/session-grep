use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, TimeZone};
use serde_json::Value;

use super::open_readonly;
use crate::model::{Agent, Message, Role, Session, Source, one_line};

fn db() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".local/share/devin/cli/sessions.db")
}

pub fn discover(since: DateTime<Local>) -> Result<Vec<Result<Session>>> {
    let db = db();
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_readonly(&db)?;
    let mut stmt = conn.prepare(
        "SELECT id, working_directory, title, last_activity_at FROM sessions \
         WHERE hidden = 0 AND last_activity_at >= ?1",
    )?;
    let rows = stmt.query_map([since.timestamp()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut sessions = Vec::new();
    for row in rows {
        let (id, cwd, title, activity) = row?;
        let Some(modified) = Local.timestamp_opt(activity, 0).single() else {
            sessions.push(Err(anyhow!("{id}: bad last_activity_at {activity}")));
            continue;
        };
        sessions.push(Ok(Session {
            agent: Agent::Devin,
            id: id.clone(),
            cwd,
            title: one_line(&title.unwrap_or_default(), 160),
            modified,
            source: Source::Row {
                db: db.clone(),
                session_id: id,
            },
        }));
    }
    Ok(sessions)
}

pub fn transcript(db: &Path, session_id: &str) -> Result<Vec<Message>> {
    let conn = open_readonly(db)?;
    let mut stmt = conn.prepare(TRANSCRIPT)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in stmt.query_map([session_id], |row| row.get::<_, String>(0))? {
        let value: Value = serde_json::from_str(&raw?)?;
        let role = Role::parse(value["role"].as_str().unwrap_or(""));
        let mut found = Vec::new();
        if let Some(text) = value["content"].as_str().map(str::trim)
            && !text.is_empty()
        {
            found.push(Message::new(role, text));
        }
        for call in value["tool_calls"].as_array().into_iter().flatten() {
            let name = call["name"].as_str().unwrap_or("tool");
            found.push(Message::new(
                Role::Tool,
                format!("{name}({})", call["arguments"]),
            ));
        }
        out.extend(found.into_iter().filter(|m| seen.insert(m.clone())));
    }
    Ok(out)
}

/// Devin stores a tree. Each resume and each context compaction starts a new
/// root and replays the earlier messages, so the transcript keeps the first
/// copy of each message. A summarizer run is a separate root that holds only
/// a copy of the conversation, so its subtree is skipped.
const TRANSCRIPT: &str = "
WITH RECURSIVE summarizer(node_id) AS (
    SELECT node_id FROM message_nodes
    WHERE session_id = ?1 AND parent_node_id IS NULL
      AND trim(json_extract(chat_message, '$.content'), char(32, 10, 9)) LIKE 'You are a Summarizer%'
    UNION
    SELECT m.node_id FROM message_nodes m
    JOIN summarizer s ON m.session_id = ?1 AND m.parent_node_id = s.node_id
)
SELECT chat_message FROM message_nodes
WHERE session_id = ?1 AND node_id NOT IN summarizer
ORDER BY node_id";
