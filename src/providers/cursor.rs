use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, TimeZone};
use serde_json::Value;

use super::{json, open_readonly};
use crate::model::{Agent, Message, Role, Session, Source, one_line};

fn root() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".cursor/chats")
}

/// Layout: `<root>/<workspace hash>/<chat id>/{meta.json, prompt_history.json, store.db}`.
pub fn discover(since: DateTime<Local>) -> Result<Vec<Result<Session>>> {
    let root = root();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut sessions = Vec::new();
    for workspace in std::fs::read_dir(&root)? {
        let workspace = workspace?.path();
        if !workspace.is_dir() {
            continue;
        }
        for chat in std::fs::read_dir(&workspace)? {
            let chat = chat?.path();
            let meta = chat.join("meta.json");
            // A chat without a database holds no conversation yet. The meta
            // file time is a cheap first filter, because Cursor rewrites it on
            // each update.
            if !chat.join("store.db").exists() || !changed_since(&meta, since) {
                continue;
            }
            let session =
                read_chat(&chat).with_context(|| format!("cannot read {}", meta.display()));
            if session.as_ref().is_ok_and(|s| s.modified < since) {
                continue;
            }
            sessions.push(session);
        }
    }
    Ok(sessions)
}

fn changed_since(path: &Path, since: DateTime<Local>) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|time| DateTime::<Local>::from(time) >= since)
}

fn read_chat(dir: &Path) -> Result<Session> {
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?)?;
    let updated = meta["updatedAtMs"].as_i64().context("no updatedAtMs")?;
    let title = match meta["title"].as_str() {
        Some(title) => title.to_string(),
        None => first_prompt(dir)?,
    };
    Ok(Session {
        agent: Agent::Cursor,
        id: dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        cwd: meta["cwd"].as_str().unwrap_or_default().to_string(),
        title,
        modified: Local
            .timestamp_millis_opt(updated)
            .single()
            .context("bad updatedAtMs")?,
        source: Source::Database(dir.join("store.db")),
    })
}

/// Older chats have no title in `meta.json`. Their first prompt is the title.
fn first_prompt(dir: &Path) -> Result<String> {
    let path = dir.join("prompt_history.json");
    if !path.exists() {
        return Ok(String::new());
    }
    let prompts: Vec<String> = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    Ok(prompts
        .first()
        .map(|p| one_line(p, 160))
        .unwrap_or_default())
}

/// Cursor stores each message as a content-addressed JSON blob. Protobuf blobs
/// beside them hold the tree order, so this uses insertion order instead.
pub fn transcript(db: &Path) -> Result<Vec<Message>> {
    let conn = open_readonly(db)?;
    let mut stmt = conn.prepare("SELECT data FROM blobs ORDER BY rowid")?;
    let mut out = Vec::new();
    for data in stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
        let data = data?;
        if data.first() != Some(&b'{') {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&data) else {
            continue;
        };
        let role = Role::parse(value["role"].as_str().unwrap_or(""));
        json::content_messages(role, &value["content"], &mut out);
    }
    Ok(out)
}
