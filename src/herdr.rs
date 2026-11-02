//! Open a session in the Herdr layout instead of in the sgrep pane.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::model::{Session, Source};
use crate::open;

/// Where Enter opens the session.
#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    /// This agent pane already runs the session.
    Focus { pane_id: String },
    /// A pane of this workspace is in the session folder.
    NewTab { workspace_id: String },
}

/// One pane of `herdr pane list`.
#[derive(Debug)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub cwd: String,
    pub foreground_cwd: String,
    /// The session file or the session id that the agent of this pane runs.
    pub agent_session: Option<String>,
}

pub fn inside() -> bool {
    std::env::var("HERDR_ENV").is_ok_and(|v| v == "1")
}

/// A pane that runs the session wins over a pane in its folder. The sgrep
/// pane itself is never a folder match: then the session opens in place.
pub fn target(session: &Session, panes: &[Pane], own_pane: &str) -> Option<Target> {
    let path = match &session.source {
        Source::File(path) => path.to_str(),
        _ => None,
    };
    let runs_session = |pane: &&Pane| {
        pane.agent_session
            .as_deref()
            .is_some_and(|value| value == session.id || Some(value) == path)
    };
    if let Some(pane) = panes.iter().find(runs_session) {
        return Some(Target::Focus {
            pane_id: pane.pane_id.clone(),
        });
    }
    panes
        .iter()
        .find(|pane| {
            pane.pane_id != own_pane
                && !session.cwd.is_empty()
                && (pane.cwd == session.cwd || pane.foreground_cwd == session.cwd)
        })
        .map(|pane| Target::NewTab {
            workspace_id: pane.workspace_id.clone(),
        })
}

pub fn panes() -> Result<Vec<Pane>> {
    let list = herdr(&["pane", "list"])?;
    let panes = list["result"]["panes"]
        .as_array()
        .context("herdr pane list returned no panes")?;
    Ok(panes
        .iter()
        .map(|pane| {
            let text = |key: &str| pane[key].as_str().unwrap_or_default().to_string();
            Pane {
                pane_id: text("pane_id"),
                workspace_id: text("workspace_id"),
                cwd: text("cwd"),
                foreground_cwd: text("foreground_cwd"),
                agent_session: pane["agent_session"]["value"].as_str().map(String::from),
            }
        })
        .collect())
}

pub fn open(session: &Session, target: &Target) -> Result<()> {
    match target {
        Target::Focus { pane_id } => {
            herdr(&["agent", "focus", pane_id])?;
        }
        Target::NewTab { workspace_id } => {
            herdr(&["workspace", "focus", workspace_id])?;
            let created = herdr(&[
                "tab",
                "create",
                "--workspace",
                workspace_id,
                "--cwd",
                &session.cwd,
                "--label",
                session.agent.label(),
                "--focus",
            ])?;
            let pane_id = created["result"]["root_pane"]["pane_id"]
                .as_str()
                .context("herdr tab create returned no pane")?;
            herdr(&["pane", "run", pane_id, &open::command_line(session)])?;
        }
    }
    Ok(())
}

fn herdr(args: &[&str]) -> Result<Value> {
    let output = Command::new("herdr")
        .args(args)
        .output()
        .context("cannot run herdr")?;
    if !output.status.success() {
        bail!(
            "herdr {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("herdr {} returned no JSON", args.join(" ")))
}

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;
    use crate::model::Agent;

    fn session() -> Session {
        Session {
            agent: Agent::Pi,
            id: "abc".into(),
            cwd: "/w/project".into(),
            title: String::new(),
            modified: Local::now(),
            source: Source::File("/s/a_abc.jsonl".into()),
        }
    }

    fn pane(id: &str, cwd: &str, agent_session: Option<&str>) -> Pane {
        Pane {
            pane_id: id.into(),
            workspace_id: id.split(':').next().unwrap().into(),
            cwd: cwd.into(),
            foreground_cwd: cwd.into(),
            agent_session: agent_session.map(String::from),
        }
    }

    #[test]
    fn running_session_is_focused_before_a_folder_match() {
        let panes = [
            pane("w1:p1", "/w/project", None),
            pane("w2:p1", "/elsewhere", Some("/s/a_abc.jsonl")),
        ];
        assert_eq!(
            target(&session(), &panes, "w9:p1"),
            Some(Target::Focus {
                pane_id: "w2:p1".into()
            })
        );
    }

    #[test]
    fn session_id_also_matches() {
        let panes = [pane("w2:p1", "/elsewhere", Some("abc"))];
        assert_eq!(
            target(&session(), &panes, "w9:p1"),
            Some(Target::Focus {
                pane_id: "w2:p1".into()
            })
        );
    }

    #[test]
    fn folder_match_opens_a_tab_in_that_workspace() {
        let panes = [
            pane("w1:p1", "/other", None),
            pane("w3:p2", "/w/project", None),
        ];
        assert_eq!(
            target(&session(), &panes, "w9:p1"),
            Some(Target::NewTab {
                workspace_id: "w3".into()
            })
        );
    }

    #[test]
    fn own_pane_and_other_folders_do_not_match() {
        let panes = [
            pane("w9:p1", "/w/project", None),
            pane("w1:p1", "/w/project-2", None),
        ];
        assert_eq!(target(&session(), &panes, "w9:p1"), None);
    }
}
