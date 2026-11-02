//! Terminal labels supplement conversation titles without replacing them.

use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde_json::Value;

use crate::model::{Agent, Session, Source};

pub struct Names {
    /// The agent's session ID or transcript path, never a terminal ID.
    pub identity: String,
    pub names: Vec<String>,
    pub pane_name: Option<String>,
    pub tab_name: Option<String>,
}

impl Names {
    pub fn new(identity: String, pane: &str, tab: &str) -> Self {
        Self {
            identity,
            names: custom_names([pane, tab]),
            pane_name: custom_names([pane]).into_iter().next(),
            tab_name: custom_names([tab]).into_iter().next(),
        }
    }
}

pub fn refresh(sessions: &mut [Session]) -> Result<()> {
    let cache_path = dirs::cache_dir().map(|path| path.join("sgrep/terminal-names.json"));
    let mut cached: HashMap<String, Vec<String>> = cache_path
        .as_ref()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    for session in sessions.iter_mut() {
        session.names = cached.get(&key(session)).cloned().unwrap_or_default();
        session.pane_name = cached
            .get(&format!("{}:pane", key(session)))
            .and_then(|names| names.first())
            .cloned();
        session.tab_name = cached
            .get(&format!("{}:tab", key(session)))
            .and_then(|names| names.first())
            .cloned();
    }
    let live = if crate::rex::inside() {
        crate::rex::names(sessions)?
    } else if crate::herdr::inside() {
        crate::herdr::names()?
    } else {
        return Ok(());
    };
    apply(sessions, &live);
    for session in sessions {
        if session.names.is_empty() {
            cached.remove(&key(session));
        } else {
            cached.insert(key(session), session.names.clone());
        }
        for (kind, name) in [("pane", &session.pane_name), ("tab", &session.tab_name)] {
            let key = format!("{}:{kind}", key(session));
            if let Some(name) = name {
                cached.insert(key, vec![name.clone()]);
            } else {
                cached.remove(&key);
            }
        }
    }
    if let Some(path) = cache_path {
        fs::create_dir_all(path.parent().expect("cache has a parent"))?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, serde_json::to_vec(&cached)?)?;
        fs::rename(&temporary, &path).context("cannot save terminal names")?;
    }
    Ok(())
}

fn key(session: &Session) -> String {
    format!("{}:{}", session.agent.label(), session.id)
}

pub fn apply(sessions: &mut [Session], live: &[Names]) {
    for session in sessions {
        let mut names = Vec::new();
        let mut matched = false;
        let mut pane_name = None;
        let mut tab_name = None;
        for entry in live
            .iter()
            .filter(|entry| identifies(session, &entry.identity))
        {
            matched = true;
            pane_name = pane_name.or_else(|| entry.pane_name.clone());
            tab_name = tab_name.or_else(|| entry.tab_name.clone());
            for name in &entry.names {
                let name = name.trim();
                if !name.is_empty() && !names.iter().any(|old| old == name) {
                    names.push(name.to_string());
                }
            }
        }
        if matched {
            session.names = names;
            session.pane_name = pane_name;
            session.tab_name = tab_name;
        }
    }
}

pub fn identifies(session: &Session, identity: &str) -> bool {
    !identity.is_empty()
        && (session.id == identity
            || matches!(&session.source, Source::File(path) if path.to_str() == Some(identity)))
}

pub fn custom_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut result = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty()
            || name.starts_with("Window ")
            || matches!(
                name.to_ascii_lowercase().as_str(),
                "shell"
                    | "terminal"
                    | "pi"
                    | "codex"
                    | "claude"
                    | "cursor"
                    | "cursor-agent"
                    | "agy"
                    | "devin"
            )
            || result.iter().any(|old| old == name)
        {
            continue;
        }
        result.push(name.to_string());
    }
    result
}

/// A resume flag is direct evidence. For interactive launches, use recorded
/// `rex whoami` results from tool output, choosing the latest observation.
pub fn resolve_rex<'a>(
    sessions: &'a [Session],
    agent: Agent,
    cwd: &str,
    command: &str,
    block: &str,
) -> Option<&'a Session> {
    let candidates: Vec<_> = sessions
        .iter()
        .filter(|session| session.agent == agent && crate::rex::same_directory(&session.cwd, cwd))
        .collect();
    if let Some(session) = candidates
        .iter()
        .find(|session| command_identifies(session, command))
    {
        return Some(*session);
    }
    let observed: Vec<_> = candidates
        .into_iter()
        .filter_map(|session| observed_block(session, block).map(|time| (session, time)))
        .collect();
    let latest = observed.iter().map(|(_, time)| *time).max()?;
    let mut matching = observed.into_iter().filter(|(_, time)| *time == latest);
    let session = matching.next()?.0;
    matching.next().is_none().then_some(session)
}

fn command_identifies(session: &Session, command: &str) -> bool {
    let flag = match session.agent {
        Agent::Pi => "--session",
        Agent::Codex => "resume",
        Agent::Claude | Agent::Cursor => "--resume",
        Agent::Agy => "--conversation",
        Agent::Devin => "-r",
    };
    let mut identities = vec![session.id.as_str()];
    if let Source::File(path) = &session.source
        && let Some(path) = path.to_str()
    {
        identities.push(path);
    }
    identities.into_iter().any(|identity| {
        !identity.is_empty()
            && Regex::new(&format!(
                r#"(?:^|\s){}(?:\s+|=)['\"]?{}(?:['\"]?(?:\s|$))"#,
                regex::escape(flag),
                regex::escape(identity)
            ))
            .expect("escaped arguments form a valid pattern")
            .is_match(command)
    })
}

fn observed_block(session: &Session, block: &str) -> Option<DateTime<Utc>> {
    let Source::File(path) = &session.source else {
        return None;
    };
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .filter(|line| line.contains(block))
        .filter_map(|line| {
            let record: Value = serde_json::from_str(line).ok()?;
            observation(&record, block)
        })
        .max()
}

fn observation(record: &Value, block: &str) -> Option<DateTime<Utc>> {
    let output = if record["message"]["role"] == "toolResult" {
        record["message"]["content"].clone()
    } else if matches!(
        record["payload"]["type"].as_str(),
        Some("function_call_output" | "custom_tool_call_output")
    ) {
        record["payload"]["output"].clone()
    } else {
        let parts = record["message"]["content"].as_array()?;
        Value::Array(
            parts
                .iter()
                .filter(|part| part["type"] == "tool_result")
                .cloned()
                .collect(),
        )
    };
    if !contains_whoami(&output, block, 0) {
        return None;
    }
    record["timestamp"].as_str()?.parse::<DateTime<Utc>>().ok()
}

fn contains_whoami(value: &Value, block: &str, depth: usize) -> bool {
    if depth > 12 {
        return false;
    }
    match value {
        Value::Object(fields) => {
            (value["block_id"] == block
                && value["principal"].is_object()
                && value["session_id"].is_string())
                || fields
                    .values()
                    .any(|value| contains_whoami(value, block, depth + 1))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| contains_whoami(value, block, depth + 1)),
        Value::String(text) if text.contains(block) => text.match_indices('{').any(|(at, _)| {
            serde_json::Deserializer::from_str(&text[at..])
                .into_iter::<Value>()
                .next()
                .and_then(Result::ok)
                .is_some_and(|value| contains_whoami(&value, block, depth + 1))
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session(id: &str) -> Session {
        Session {
            agent: Agent::Pi,
            id: id.into(),
            cwd: "/work/project".into(),
            title: "Original prompt".into(),
            names: Vec::new(),
            pane_name: None,
            tab_name: None,
            modified: chrono::Local::now(),
            source: Source::File(format!("/sessions/{id} file.jsonl").into()),
        }
    }

    #[test]
    fn names_match_identity_and_preserve_original_title() {
        let mut sessions = vec![session("first"), session("second")];
        apply(
            &mut sessions,
            &[
                Names::new("first".into(), "Fix login", ""),
                Names::new("/sessions/first file.jsonl".into(), "Fix login", "Review"),
            ],
        );
        assert_eq!(sessions[0].names, ["Fix login", "Review"]);
        assert_eq!(sessions[0].pane_name.as_deref(), Some("Fix login"));
        assert_eq!(sessions[0].tab_name.as_deref(), Some("Review"));
        assert_eq!(sessions[0].resume_label(), "Review");
        assert_eq!(sessions[0].title, "Original prompt");
        assert!(sessions[0].haystack().contains("fix login"));
        assert_eq!(
            crate::search::metadata_hits(&sessions, "pi login review"),
            [0]
        );
        assert!(sessions[1].names.is_empty());
        apply(&mut sessions, &[Names::new("first".into(), "", "")]);
        assert!(sessions[0].names.is_empty());
        assert_eq!(sessions[0].pane_name, None);
        assert_eq!(sessions[0].tab_name, None);
    }

    #[test]
    fn resume_arguments_match_exact_ids_and_paths_with_spaces() {
        let session = session("abc");
        for command in [
            "pi --session abc",
            "pi --session=abc --model test",
            "pi --session '/sessions/abc file.jsonl'",
        ] {
            assert!(command_identifies(&session, command), "{command}");
        }
        for command in ["pi --session abc-other", "pi --model abc", "pi"] {
            assert!(!command_identifies(&session, command), "{command}");
        }
        assert!(
            resolve_rex(
                &[session],
                Agent::Pi,
                "/work/project",
                "pi",
                "block:missing"
            )
            .is_none()
        );
    }

    #[test]
    fn only_tool_observations_can_establish_terminal_identity() {
        let whoami = json!({"principal": {"kind":"peer"}, "session_id":"session:one", "block_id":"block:one"});
        let output = format!("stdout:\n{whoami}\n");
        let pi = json!({"timestamp":"2026-11-02T01:00:00Z", "message":{"role":"toolResult", "content":[{"type":"text", "text": output}]}});
        let codex = json!({"timestamp":"2026-11-02T01:00:00Z", "payload":{"type":"function_call_output", "output": output}});
        let claude = json!({"timestamp":"2026-11-02T01:00:00Z", "message":{"role":"user", "content":[{"type":"tool_result", "content":output}]}});
        for record in [pi.clone(), codex, claude] {
            assert!(observation(&record, "block:one").is_some());
            assert!(observation(&record, "block:other").is_none());
        }
        let mut user = pi;
        user["message"]["role"] = json!("user");
        assert!(observation(&user, "block:one").is_none());
        assert!(!contains_whoami(
            &json!({"block_id":"block:one"}),
            "block:one",
            0
        ));
    }

    #[test]
    fn ambiguous_copied_history_is_not_assigned_to_a_session() {
        let root = std::env::temp_dir().join(format!(
            "sgrep-names-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let mut sessions = vec![session("first"), session("second")];
        let mut record = json!({
            "timestamp":"2026-11-02T01:00:00Z",
            "message":{"role":"toolResult", "content":[{"type":"text", "text":
                json!({"principal":{}, "session_id":"session:one", "block_id":"block:one"}).to_string()
            }]}
        });
        for (index, session) in sessions.iter_mut().enumerate() {
            let path = root.join(format!("{index}.jsonl"));
            fs::write(&path, record.to_string()).unwrap();
            session.source = Source::File(path);
        }
        assert!(resolve_rex(&sessions, Agent::Pi, "/work/project", "pi", "block:one").is_none());
        record["timestamp"] = json!("2026-11-02T02:00:00Z");
        fs::write(root.join("1.jsonl"), record.to_string()).unwrap();
        let matched =
            resolve_rex(&sessions, Agent::Pi, "/work/project", "pi", "block:one").unwrap();
        assert_eq!(matched.id, "second");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_labels_are_omitted_and_custom_labels_are_deduplicated() {
        assert_eq!(
            custom_names([
                "shell",
                "Window 1234",
                "pi",
                " Login fix ",
                "Login fix",
                "Review"
            ]),
            ["Login fix", "Review"]
        );
    }
}
