use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::thread;

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};

use crate::model::{Agent, Message, Role, Session};
use crate::providers;

/// The conversation of each session, loaded on first use and shared by the
/// preview and the content search. A failed load keeps its error text for the
/// preview.
pub struct Transcripts {
    sessions: Arc<Vec<Session>>,
    slots: Vec<OnceLock<Result<Vec<Message>, String>>>,
}

impl Transcripts {
    pub fn new(sessions: Arc<Vec<Session>>) -> Transcripts {
        let slots = (0..sessions.len()).map(|_| OnceLock::new()).collect();
        Transcripts { sessions, slots }
    }

    /// Parse every transcript in the background, so that the first content
    /// search does not wait for the parse.
    pub fn warm(self: &Arc<Self>) {
        let transcripts = Arc::clone(self);
        thread::spawn(move || {
            (0..transcripts.slots.len())
                .into_par_iter()
                .for_each(|index| {
                    transcripts.get(index);
                })
        });
    }

    pub fn get(&self, index: usize) -> &Result<Vec<Message>, String> {
        self.slots[index].get_or_init(|| {
            providers::transcript(&self.sessions[index])
                .map(conversation)
                .map_err(|error| format!("{error:#}"))
        })
    }
}

/// Result of one finished background content scan.
pub struct ContentHits {
    pub generation: u64,
    pub hits: Vec<usize>,
}

/// Runs full-content scans off the UI thread. A new scan makes the running
/// one stop and discard its result.
pub struct ContentSearch {
    transcripts: Arc<Transcripts>,
    generation: Arc<AtomicU64>,
    tx: Sender<ContentHits>,
}

impl ContentSearch {
    pub fn new(transcripts: Arc<Transcripts>, tx: Sender<ContentHits>) -> ContentSearch {
        ContentSearch {
            transcripts,
            generation: Arc::new(AtomicU64::new(0)),
            tx,
        }
    }

    pub fn cancel(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn start(&self, query: &str) -> u64 {
        let generation = self.cancel();
        let query = Query::new(query);
        let transcripts = Arc::clone(&self.transcripts);
        let current = Arc::clone(&self.generation);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let live = || current.load(Ordering::SeqCst) == generation;
            let hits: Vec<usize> = (0..transcripts.slots.len())
                .into_par_iter()
                .filter(|&index| {
                    if !live() {
                        return false;
                    }
                    let session = &transcripts.sessions[index];
                    query.accepts_agent(session)
                        && (query.matches_metadata(session)
                            || query.matches_session(session, transcripts.get(index)))
                })
                .collect();
            if live() {
                // The receiver is gone only when the app quits.
                let _ = tx.send(ContentHits { generation, hits });
            }
        });
        generation
    }
}

/// A literal, case-insensitive pattern. The query is not a regex.
pub fn pattern(query: &str) -> Regex {
    RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
        .expect("an escaped literal is a valid regex")
}

/// Only the user and assistant text. The preview hides the tool calls and the
/// thinking, so the search ignores them too: each hit is visible.
fn conversation(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .filter(|m| matches!(m.role, Role::User | Role::Assistant))
        .collect()
}

fn matches(transcript: &Result<Vec<Message>, String>, pattern: &Regex) -> bool {
    transcript
        .as_ref()
        .is_ok_and(|messages| messages.iter().any(|m| pattern.is_match(&m.text)))
}

/// A leading agent name filters the provider. Whitespace separates required
/// literal terms; a standalone `+` is optional. Terms can match different
/// metadata fields or different conversation turns.
pub struct Query {
    agent: Option<Agent>,
    terms: Vec<Regex>,
}

impl Query {
    pub fn new(query: &str) -> Self {
        let mut words = query
            .split_whitespace()
            .filter(|term| *term != "+")
            .peekable();
        let agent = words
            .peek()
            .and_then(|word| match word.to_ascii_lowercase().as_str() {
                "pi" => Some(Agent::Pi),
                "codex" => Some(Agent::Codex),
                "claude" => Some(Agent::Claude),
                "cursor" | "cursor-agent" => Some(Agent::Cursor),
                "agy" => Some(Agent::Agy),
                "devin" => Some(Agent::Devin),
                _ => None,
            });
        if agent.is_some() {
            words.next();
        }
        Self {
            agent,
            terms: words.map(pattern).collect(),
        }
    }

    pub fn agent(&self) -> Option<Agent> {
        self.agent
    }

    fn accepts_agent(&self, session: &Session) -> bool {
        self.agent.is_none_or(|agent| session.agent == agent)
    }

    pub fn matches_metadata(&self, session: &Session) -> bool {
        let metadata = session.haystack();
        self.accepts_agent(session) && self.terms.iter().all(|term| term.is_match(&metadata))
    }

    fn matches_session(
        &self,
        session: &Session,
        transcript: &Result<Vec<Message>, String>,
    ) -> bool {
        let metadata = session.haystack();
        self.accepts_agent(session)
            && self
                .terms
                .iter()
                .all(|term| term.is_match(&metadata) || matches(transcript, term))
    }

    pub fn highlight_pattern(&self) -> Option<Regex> {
        if self.terms.is_empty() {
            return None;
        }
        let alternatives = self
            .terms
            .iter()
            .map(|term| term.as_str())
            .collect::<Vec<_>>()
            .join("|");
        Some(
            RegexBuilder::new(&alternatives)
                .case_insensitive(true)
                .build()
                .expect("escaped literal alternatives form a valid regex"),
        )
    }
}

/// Case-insensitive matching of every term against the session metadata.
pub fn metadata_hits(sessions: &[Session], query: &str) -> Vec<usize> {
    let query = Query::new(query);
    sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| query.matches_metadata(session))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(agent: Agent, title: &str) -> Session {
        Session {
            agent,
            id: "test-session".into(),
            cwd: "/work/project".into(),
            title: title.into(),
            names: Vec::new(),
            pane_name: None,
            tab_name: None,
            modified: chrono::Local::now(),
            source: crate::model::Source::File("/missing/session.jsonl".into()),
        }
    }

    #[test]
    fn agent_and_content_match_across_fields() {
        let devin = session(Agent::Devin, "Fix login");
        let codex = session(Agent::Codex, "Devin sentence");
        let transcript = Ok(vec![Message::new(
            Role::Assistant,
            "A SENTENCE in the body",
        )]);
        for text in ["devin sentence", "Devin + sentence", "devin login sentence"] {
            let query = Query::new(text);
            assert!(query.matches_session(&devin, &transcript), "{text}");
            assert!(!query.matches_session(&codex, &transcript), "{text}");
            assert!(!query.matches_metadata(&devin), "{text}");
        }
        assert!(Query::new("devin").matches_metadata(&devin));
        assert!(!Query::new("devin missing").matches_session(&devin, &transcript));
    }

    #[test]
    fn terms_can_match_separate_messages_and_metadata() {
        let session = session(Agent::Pi, "Update login");
        let transcript = Ok(vec![
            Message::new(Role::User, "修复权限"),
            Message::new(Role::Assistant, "Add regression coverage"),
        ]);
        assert!(Query::new("login 权限 coverage").matches_session(&session, &transcript));
        assert!(!Query::new("login 权限 missing").matches_session(&session, &transcript));
        assert_eq!(metadata_hits(&[session], "pi project login"), [0]);
    }

    #[test]
    fn search_terms_keep_literal_punctuation() {
        let session = session(Agent::Devin, "C++ a.b a+b");
        assert!(Query::new("devin + C++ a.b a+b").matches_metadata(&session));
        assert!(!Query::new("devin axb").matches_metadata(&session));
        assert!(Query::new("  +  ").matches_metadata(&session));
        let highlight = Query::new("devin + a.b C++").highlight_pattern().unwrap();
        assert!(highlight.is_match("a.b and C++"));
        assert!(!highlight.is_match("devin axb"));
    }

    #[test]
    fn background_search_combines_agent_metadata_and_content() {
        use std::sync::mpsc::channel;
        use std::time::Duration;

        let sessions = Arc::new(vec![
            session(Agent::Devin, "Fix login"),
            session(Agent::Codex, "Devin sentence"),
            session(Agent::Devin, "Other work"),
        ]);
        let transcripts = Arc::new(Transcripts::new(sessions));
        for (slot, text) in transcripts.slots.iter().zip([
            "The required sentence",
            "The required sentence",
            "No matching content",
        ]) {
            slot.set(Ok(vec![Message::new(Role::Assistant, text)]))
                .unwrap();
        }
        let (tx, rx) = channel();
        let search = ContentSearch::new(transcripts, tx);
        let generation = search.start("devin + sentence");
        let result = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(result.generation, generation);
        assert_eq!(result.hits, [0]);
    }

    #[test]
    fn conversation_keeps_only_user_and_assistant() {
        let kept = conversation(vec![
            Message::new(Role::System, "rules"),
            Message::new(Role::User, "ask"),
            Message::new(Role::Thinking, "plan"),
            Message::new(Role::Tool, "ls()"),
            Message::new(Role::Assistant, "answer"),
        ]);
        assert_eq!(
            kept,
            [
                Message::new(Role::User, "ask"),
                Message::new(Role::Assistant, "answer")
            ]
        );
    }

    #[test]
    fn content_match_is_case_insensitive() {
        let found = Ok(vec![Message::new(Role::User, "find the NEEDLE")]);
        assert!(matches(&found, &pattern("Needle")));
        assert!(!matches(&found, &pattern("other")));
    }

    #[test]
    fn query_is_literal() {
        assert!(pattern("a.b").is_match("A.B"));
        assert!(!pattern("a.b").is_match("axb"));
    }
}
