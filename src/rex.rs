//! Resume an agent in a new tab of a Rex session with the same directory.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::model::{Agent, Session};
use crate::names::{self, Names};

pub fn inside() -> bool {
    std::env::var("REX_SESSION").is_ok_and(|value| !value.is_empty())
}

pub fn names(sessions: &[Session]) -> Result<Vec<Names>> {
    let server = rex(&["whoami", "--json"])?;
    let host = server["host_id"]
        .as_str()
        .context("rex whoami returned no host")?;
    let listed = rex(&["ls", "--json"])?;
    let listed = listed["sessions"]
        .as_array()
        .context("rex ls returned no sessions")?;
    let mut result = Vec::new();
    for item in listed {
        let Some(id) = item["session_id"].as_str() else {
            continue;
        };
        let Ok(view) = rex(&["window", "ls", "-s", id, "--json"]) else {
            continue;
        };
        let Some(windows) = view["windows"].as_array() else {
            continue;
        };
        for window in windows {
            for layer in window["layers"].as_array().into_iter().flatten() {
                for block in layer["blocks"].as_array().into_iter().flatten() {
                    if block["creator_name"] != "com.superlogical.terminal"
                        || block["host_id"] != host
                    {
                        continue;
                    }
                    let Some(block_id) = block["block_id"].as_str() else {
                        continue;
                    };
                    let Ok(process) = rex(&["block", "call", "-s", id, "-b", block_id, "process"])
                    else {
                        continue;
                    };
                    let foreground = &process["foreground"];
                    let Some(agent) = process_agent(foreground) else {
                        continue;
                    };
                    let Some(cwd) = foreground["cwd"].as_str() else {
                        continue;
                    };
                    let command = foreground["pid"]
                        .as_u64()
                        .and_then(|pid| {
                            Command::new("ps")
                                .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
                                .output()
                                .ok()
                                .filter(|output| output.status.success())
                                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        })
                        .unwrap_or_default();
                    if let Some(session) =
                        names::resolve_rex(sessions, agent, cwd, &command, block_id)
                    {
                        result.push(Names::new(
                            session.id.clone(),
                            block["label"].as_str().unwrap_or_default(),
                            window["label"].as_str().unwrap_or_default(),
                        ));
                    }
                }
            }
        }
    }
    Ok(result)
}

fn process_agent(process: &Value) -> Option<Agent> {
    ["argv0", "name", "script"]
        .into_iter()
        .filter_map(|field| process[field].as_str())
        .find_map(|value| {
            let name = Path::new(value).file_stem()?.to_str()?;
            match name {
                "pi" => Some(Agent::Pi),
                "codex" => Some(Agent::Codex),
                "claude" => Some(Agent::Claude),
                "cursor-agent" => Some(Agent::Cursor),
                "agy" => Some(Agent::Agy),
                "devin" => Some(Agent::Devin),
                _ if value.contains("pi-coding-agent/") => Some(Agent::Pi),
                _ => None,
            }
        })
}

/// Open a new tab when a terminal in Rex has the agent session's directory.
/// Otherwise the caller resumes the agent in place.
pub fn open(session: &Session) -> Result<bool> {
    if session.cwd.is_empty() {
        return Ok(false);
    }
    let own_session = std::env::var("REX_SESSION").unwrap_or_default();
    let own_block = std::env::var("REX_BLOCK").unwrap_or_default();
    let own_blocks = rex(&["block", "ls", "-s", &own_session, "--json"])?;
    let own = own_blocks["blocks"]
        .as_array()
        .and_then(|blocks| blocks.iter().find(|block| block["block_id"] == own_block));
    let own_host = own.and_then(|block| block["host_id"].as_str());
    let own_client = own.and_then(|block| block["owner"].as_str());

    let list = rex(&["ls", "--json"])?;
    let mut sessions: Vec<&str> = list["sessions"]
        .as_array()
        .context("rex ls returned no sessions")?
        .iter()
        .filter_map(|item| item["session_id"].as_str())
        .collect();
    // Prefer the current session when several sessions have the same folder.
    sessions.sort_by_key(|id| *id != own_session);
    for id in sessions {
        let blocks = if id == own_session {
            own_blocks.clone()
        } else {
            rex(&["block", "ls", "-s", id, "--json"])?
        };
        let blocks = blocks["blocks"]
            .as_array()
            .context("rex block ls returned no blocks")?;
        for block in blocks {
            if block["creator_name"] != "com.superlogical.terminal"
                || block["placement"] != "placed"
                || own_host.is_some_and(|host| block["host_id"] != host)
            {
                continue;
            }
            let Some(block_id) = block["block_id"].as_str() else {
                continue;
            };
            // A terminal can close while the session list is being inspected.
            let Ok(process) = rex(&["block", "call", "-s", id, "-b", block_id, "process"]) else {
                continue;
            };
            if !in_directory(&process, &session.cwd) {
                continue;
            }
            let mut args = vec![
                "window".to_string(),
                "new".into(),
                "-s".into(),
                id.into(),
                session.resume_label().into(),
                "-c".into(),
                session.cwd.clone(),
                "--json".into(),
                "--".into(),
            ];
            args.extend(session.resume_command());
            let created = rex(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            let window = created["window_id"]
                .as_str()
                .context("rex window new returned no window")?;
            if let Some(block) = created["block_ids"]
                .as_array()
                .and_then(|blocks| blocks.first())
                .and_then(Value::as_str)
            {
                restore_pane_name(session, id, block);
            }
            // Creating a window already focuses it within its session. Only
            // switching to another session needs the app's Remote Control.
            if id != own_session {
                let mut select = Vec::new();
                if let Some(client) = own_client {
                    select.extend(["-C".to_string(), client.into()]);
                }
                select.extend([
                    "do".into(),
                    "session.select".into(),
                    format!("session_id={id}"),
                    format!("window_id={window}"),
                ]);
                if let Err(error) = run(&select.iter().map(String::as_str).collect::<Vec<_>>()) {
                    // The agent is already running. Do not retry or resume it
                    // in place when only the app's navigation failed.
                    eprintln!(
                        "sgrep: opened {} in Rex session {id}, tab {window}, but could not switch to it: {error}\n\
                         Select the tab manually. To allow automatic switching, enable Remote Control in Rex Settings > Rex Server.",
                        session.agent.label()
                    );
                }
            }
            return Ok(true);
        }
    }
    if let Some(own) = own {
        restore_pane_name(session, &own_session, &own_block);
        if (!session.names.is_empty() || session.tab_name.is_some() || session.pane_name.is_some())
            && let Some(window) = own["window_id"].as_str()
            && let Err(error) = run(&[
                "window",
                "rename",
                "-s",
                &own_session,
                window,
                session.resume_label(),
            ])
        {
            eprintln!("sgrep: cannot restore tab name: {error}");
        }
    }
    Ok(false)
}

fn restore_pane_name(session: &Session, target: &str, block: &str) {
    if let Some(name) = session.pane_name.as_deref()
        && let Err(error) = run(&["block", "rename", "-s", target, block, name])
    {
        eprintln!("sgrep: cannot restore pane name: {error}");
    }
}

fn in_directory(process: &Value, cwd: &str) -> bool {
    // Include the shell's directory when its foreground command changed folders.
    std::iter::once(&process["foreground"])
        .chain(process["ancestors"].as_array().into_iter().flatten())
        .chain(std::iter::once(&process["child"]))
        .filter_map(|process| process["cwd"].as_str())
        .any(|dir| same_directory(dir, cwd))
}

pub(crate) fn same_directory(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let left = Path::new(left);
    let right = Path::new(right);
    left == right
        || left
            .canonicalize()
            .ok()
            .zip(right.canonicalize().ok())
            .is_some_and(|(left, right)| left == right)
}

fn rex(args: &[&str]) -> Result<Value> {
    serde_json::from_slice(&run(args)?)
        .with_context(|| format!("rex {} returned no JSON", args.join(" ")))
}

fn run(args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("rex")
        .args(args)
        .output()
        .context("cannot run rex")?;
    if !output.status.success() {
        bail!(
            "rex {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn matches_foreground_or_shell_directory() {
        let process = json!({
            "foreground": { "cwd": "/work/project/subdir" },
            "ancestors": [{ "cwd": "/work/project" }],
        });
        assert!(in_directory(&process, "/work/project/subdir"));
        assert!(in_directory(&process, "/work/project"));
        assert!(!in_directory(&process, "/work/project-other"));
        assert!(!in_directory(&process, ""));
    }

    #[test]
    fn missing_process_directory_does_not_match() {
        assert!(!in_directory(
            &json!({ "foreground": null }),
            "/work/project"
        ));
    }

    #[test]
    fn equivalent_paths_match() {
        let cwd = std::env::current_dir().unwrap();
        assert!(same_directory(
            cwd.to_str().unwrap(),
            cwd.join("src/..").to_str().unwrap()
        ));
        assert!(same_directory("/work/project/", "/work/project"));
    }
}
