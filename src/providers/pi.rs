use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Local};
use serde_json::Value;

use super::{json, jsonl};
use crate::model::{Agent, Message, Role, Session, Source};

fn root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".pi/agent/sessions")
}

pub fn discover(since: DateTime<Local>) -> Result<Vec<Result<Session>>> {
    jsonl::discover(&root(), since, |found, head| {
        let header = head.iter().find(|r| r["type"] == "session");
        let field = |name: &str| {
            header
                .and_then(|r| r[name].as_str())
                .unwrap_or_default()
                .to_string()
        };
        Session {
            agent: Agent::Pi,
            id: field("id"),
            cwd: field("cwd"),
            title: json::title_of(&messages(&head)),
            names: Vec::new(),
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
    for record in records.iter().filter(|r| r["type"] == "message") {
        let message = &record["message"];
        let role = Role::parse(message["role"].as_str().unwrap_or(""));
        json::content_messages(role, &message["content"], &mut out);
    }
    out
}
