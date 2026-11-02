use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use regex::Regex;

use super::{open_readonly, protobuf};
use crate::model::{Agent, Message, Role, Session, Source, one_line};

fn home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".gemini/antigravity-cli")
}

pub fn discover(since: DateTime<Local>) -> Result<Vec<Result<Session>>> {
    let summaries = home().join("conversation_summaries.db");
    if !summaries.exists() {
        return Ok(Vec::new());
    }
    let conn = open_readonly(&summaries)?;
    let mut stmt = conn.prepare(
        "SELECT conversation_id, title, preview, last_modified_time, workspace_uris \
         FROM conversation_summaries",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut sessions = Vec::new();
    for row in rows {
        let (id, title, preview, modified, workspaces) = row?;
        let Some(modified) = parse_time(&modified) else {
            sessions.push(Err(anyhow!("{id}: bad last_modified_time {modified:?}")));
            continue;
        };
        let db = home().join("conversations").join(format!("{id}.db"));
        if modified < since || !db.exists() {
            continue;
        }
        sessions.push(Ok(Session {
            agent: Agent::Agy,
            title: if title.is_empty() {
                one_line(&preview, 160)
            } else {
                title
            },
            id,
            cwd: first_workspace(&workspaces),
            names: Vec::new(),
            pane_name: None,
            tab_name: None,
            modified,
            source: Source::Database(db),
        }));
    }
    Ok(sessions)
}

/// The column holds `2026-10-29 08:11:25.681144+00:00`.
fn parse_time(raw: &str) -> Option<DateTime<Local>> {
    let head = raw.split('+').next()?.trim();
    let naive = NaiveDateTime::parse_from_str(head, "%Y-%m-%d %H:%M:%S%.f").ok()?;
    Some(Local.from_utc_datetime(&naive))
}

/// The column holds `["file:///Users/me/project"]`.
fn first_workspace(raw: &str) -> String {
    serde_json::from_str::<Vec<String>>(raw)
        .ok()
        .and_then(|uris| uris.into_iter().next())
        .map(|uri| uri.trim_start_matches("file://").to_string())
        .unwrap_or_default()
}

/// Step type 14 is a user turn and 15 is a model turn. The other types are
/// tool results.
pub fn transcript(db: &Path) -> Result<Vec<Message>> {
    let conn = open_readonly(db)?;
    let mut stmt = conn.prepare("SELECT step_type, step_payload FROM steps ORDER BY idx")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (step_type, payload) = row?;
        let role = match step_type {
            14 => Role::User,
            15 => Role::Assistant,
            _ => Role::Tool,
        };
        let mut fields = Vec::new();
        protobuf::strings(&payload, &mut fields);
        fields.retain(|s| is_prose(s));
        fields.dedup();
        let text = fields.join("\n");
        if !text.trim().is_empty() {
            out.push(Message::new(role, text.trim()));
        }
    }
    Ok(out)
}

/// Drop the identifiers and the key names that the walker collects beside the
/// real text. Prose holds a space. An identifier does not.
fn is_prose(text: &str) -> bool {
    static UUID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("[0-9a-f]{8}-[0-9a-f]{4}-").unwrap());
    let text = text.trim();
    text.chars().count() >= 4 && text.contains(' ') && !UUID.is_match(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_summary_time() {
        let time = parse_time("2026-10-29 08:11:25.681144+00:00").unwrap();
        assert_eq!(time.naive_utc().to_string(), "2026-10-29 08:11:25.681144");
    }

    #[test]
    fn strips_file_scheme() {
        assert_eq!(first_workspace(r#"["file:///Users/me/p"]"#), "/Users/me/p");
    }

    #[test]
    fn prose_filter_drops_ids() {
        assert!(is_prose("can you use playwright now"));
        assert!(!is_prose(
            "$bcaa8a60-ee6d-4931-8292-5eb9c175bb24\"$0342b9e3"
        ));
        assert!(!is_prose("CommandLine"));
    }
}
