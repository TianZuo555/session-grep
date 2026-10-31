use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};
use std::thread;

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};

use crate::model::{Message, Role, Session};
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

    pub fn start(&self, query: &str) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let pattern = pattern(query);
        let transcripts = Arc::clone(&self.transcripts);
        let current = Arc::clone(&self.generation);
        let tx = self.tx.clone();
        thread::spawn(move || {
            let live = || current.load(Ordering::SeqCst) == generation;
            let hits: Vec<usize> = (0..transcripts.slots.len())
                .into_par_iter()
                .filter(|&index| live() && matches(transcripts.get(index), &pattern))
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

/// Case-insensitive substring match over the session metadata.
pub fn metadata_hits(sessions: &[Session], query: &str) -> Vec<usize> {
    let needle = query.to_lowercase();
    sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| session.haystack().contains(&needle))
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
