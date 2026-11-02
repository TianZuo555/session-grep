use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Local};
use serde_json::Value;

use super::{json, jsonl};
use crate::model::{Agent, Message, Role, Session, Source};

fn root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude/projects")
}

pub fn discover(since: DateTime<Local>) -> Result<Vec<Result<Session>>> {
    jsonl::discover(&root(), since, |found, head| Session {
        agent: Agent::Claude,
        id: found
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string(),
        cwd: head
            .iter()
            .find_map(|r| r["cwd"].as_str())
            .unwrap_or_default()
            .to_string(),
        title: json::title_of(&messages(&head)),
        names: Vec::new(),
        pane_name: None,
        tab_name: None,
        modified: found.modified,
        source: Source::File(found.path),
    })
}

pub fn transcript(path: &Path) -> Result<Vec<Message>> {
    Ok(messages(&jsonl::all(path)?))
}

fn messages(records: &[Value]) -> Vec<Message> {
    let mut out = Vec::new();
    for record in records {
        let kind = record["type"].as_str().unwrap_or("");
        if (kind != "user" && kind != "assistant") || record["isSidechain"] == Value::Bool(true) {
            continue;
        }
        let message = &record["message"];
        let role = Role::parse(message["role"].as_str().unwrap_or(kind));
        json::content_messages(role, &message["content"], &mut out);
    }
    out
}
