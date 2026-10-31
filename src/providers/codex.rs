use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Local};
use serde_json::Value;

use super::{json, jsonl};
use crate::model::{Agent, Message, Role, Session, Source};

fn root() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".codex/sessions")
}

pub fn discover(since: DateTime<Local>) -> Result<Vec<Result<Session>>> {
    jsonl::discover(&root(), since, |found, head| {
        let meta = head
            .iter()
            .find(|r| r["type"] == "session_meta")
            .map(|r| &r["payload"]);
        let field = |name: &str| {
            meta.and_then(|m| m[name].as_str())
                .unwrap_or_default()
                .to_string()
        };
        Session {
            agent: Agent::Codex,
            id: field("id"),
            cwd: field("cwd"),
            title: json::title_of(&messages(&head)),
            modified: found.modified,
            source: Source::File(found.path),
        }
    })
}

pub fn transcript(path: &Path) -> Result<Vec<Message>> {
    Ok(messages(&jsonl::all(path)?))
}

fn messages(records: &[Value]) -> Vec<Message> {
    let mut out = Vec::new();
    for record in records.iter().filter(|r| r["type"] == "response_item") {
        let payload = &record["payload"];
        match payload["type"].as_str().unwrap_or("") {
            "message" => {
                let role = Role::parse(payload["role"].as_str().unwrap_or(""));
                json::content_messages(role, &payload["content"], &mut out);
            }
            "reasoning" => json::content_messages(Role::Thinking, &payload["summary"], &mut out),
            // Tool calls and their output are top-level items, not message parts.
            _ => json::content_messages(Role::Tool, &Value::Array(vec![payload.clone()]), &mut out),
        }
    }
    out
}
