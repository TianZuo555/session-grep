//! Resume an agent in a new tab of a Rex session with the same directory.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::model::Session;

pub fn inside() -> bool {
    std::env::var("REX_SESSION").is_ok_and(|value| !value.is_empty())
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
                session.agent.label().into(),
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
    Ok(false)
}

fn in_directory(process: &Value, cwd: &str) -> bool {
    // Include the shell's directory when its foreground command changed folders.
    std::iter::once(&process["foreground"])
        .chain(process["ancestors"].as_array().into_iter().flatten())
        .chain(std::iter::once(&process["child"]))
        .filter_map(|process| process["cwd"].as_str())
        .any(|dir| same_directory(dir, cwd))
}

fn same_directory(left: &str, right: &str) -> bool {
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
