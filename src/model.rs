use std::path::PathBuf;

use chrono::{DateTime, Local};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Agent {
    Pi,
    Codex,
    Claude,
    Cursor,
    Agy,
    Devin,
}

impl Agent {
    pub fn label(self) -> &'static str {
        match self {
            Agent::Pi => "pi",
            Agent::Codex => "codex",
            Agent::Claude => "claude",
            Agent::Cursor => "cursor",
            Agent::Agy => "agy",
            Agent::Devin => "devin",
        }
    }
}

/// Where the bytes of a session live.
#[derive(Clone, Debug)]
pub enum Source {
    /// One JSONL file.
    File(PathBuf),
    /// One SQLite database that holds only this session.
    Database(PathBuf),
    /// One SQLite database that holds many sessions.
    Row { db: PathBuf, session_id: String },
}

#[derive(Clone, Debug)]
pub struct Session {
    pub agent: Agent,
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub modified: DateTime<Local>,
    pub source: Source,
}

impl Session {
    /// The text that the instant search filter matches against.
    pub fn haystack(&self) -> String {
        format!(
            "{} {} {} {}",
            self.agent.label(),
            self.id,
            self.cwd,
            self.title
        )
        .to_lowercase()
    }

    /// The command that reopens this session.
    pub fn resume_command(&self) -> Vec<String> {
        let id = self.id.as_str();
        let parts: Vec<&str> = match (self.agent, &self.source) {
            // A pi id resolves only inside its own project, a path resolves everywhere.
            (Agent::Pi, Source::File(path)) => vec!["pi", "--session", path.to_str().unwrap_or(id)],
            (Agent::Pi, _) => vec!["pi", "--session", id],
            (Agent::Codex, _) => vec!["codex", "resume", id],
            (Agent::Claude, _) => vec!["claude", "--resume", id],
            (Agent::Cursor, _) => vec!["cursor-agent", "--resume", id],
            (Agent::Agy, _) => vec!["agy", "--conversation", id],
            (Agent::Devin, _) => vec!["devin", "-r", id],
        };
        parts.into_iter().map(String::from).collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Role {
    User,
    Assistant,
    Thinking,
    Tool,
    /// Instructions and context that the harness injects. Hidden in the preview
    /// and skipped by the content search.
    System,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Thinking => "thinking",
            Role::Tool => "tool",
            Role::System => "system",
        }
    }

    pub fn parse(raw: &str) -> Role {
        match raw {
            "user" | "human" => Role::User,
            "assistant" | "model" => Role::Assistant,
            "tool" | "function" | "toolResult" => Role::Tool,
            _ => Role::System,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

impl Message {
    pub fn new(role: Role, text: impl Into<String>) -> Message {
        Message {
            role,
            text: text.into(),
        }
    }
}

/// Collapse a block of text to one line of at most `limit` terminal columns.
pub fn one_line(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.width() <= limit {
        return flat;
    }
    let mut out = String::new();
    let mut used = 0;
    for c in flat.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > limit {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(agent: Agent, source: Source) -> Session {
        Session {
            agent,
            id: "abc".into(),
            cwd: "/w".into(),
            title: String::new(),
            modified: Local::now(),
            source,
        }
    }

    #[test]
    fn pi_resumes_by_path() {
        let s = session(Agent::Pi, Source::File("/s/a_abc.jsonl".into()));
        assert_eq!(s.resume_command(), ["pi", "--session", "/s/a_abc.jsonl"]);
    }

    #[test]
    fn each_agent_has_its_resume_flag() {
        let file = || Source::File("/f".into());
        let cases = [
            (Agent::Codex, vec!["codex", "resume", "abc"]),
            (Agent::Claude, vec!["claude", "--resume", "abc"]),
            (Agent::Cursor, vec!["cursor-agent", "--resume", "abc"]),
            (Agent::Agy, vec!["agy", "--conversation", "abc"]),
            (Agent::Devin, vec!["devin", "-r", "abc"]),
        ];
        for (agent, expected) in cases {
            assert_eq!(session(agent, file()).resume_command(), expected);
        }
    }

    #[test]
    fn one_line_flattens_and_truncates() {
        assert_eq!(one_line("a\n  b\tc", 10), "a b c");
        assert_eq!(one_line("abcdef", 4), "abc…");
        // A CJK character fills two columns.
        assert_eq!(one_line("你好世界", 5), "你好…");
    }
}
