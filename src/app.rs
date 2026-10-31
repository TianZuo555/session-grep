use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use crate::model::Session;
use crate::search::{ContentHits, ContentSearch, Transcripts, metadata_hits};
use crate::ui::Rendered;

/// How long the query must stay unchanged before the content scan starts.
const DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Mode {
    Search,
    Preview,
}

pub struct App {
    pub sessions: Arc<Vec<Session>>,
    pub transcripts: Arc<Transcripts>,
    /// Indexes into `sessions` that pass the current query.
    pub visible: Vec<usize>,
    pub selected: usize,
    pub query: String,
    pub mode: Mode,
    /// The first visible row of the wrapped preview.
    pub scroll: usize,
    pub preview: Option<Rendered>,
    /// When true, the next draw scrolls the preview to the first match.
    pub jump_to_match: bool,
    pub pending_g: bool,
    pub scanning: bool,
    pub errors: Vec<String>,
    pub quit: bool,
    pub open: Option<usize>,
    shown: Option<usize>,
    debounce_until: Option<Instant>,
    search: ContentSearch,
    rx: Receiver<ContentHits>,
    latest: u64,
}

impl App {
    pub fn new(sessions: Vec<Session>, errors: Vec<String>) -> App {
        let sessions = Arc::new(sessions);
        let transcripts = Arc::new(Transcripts::new(Arc::clone(&sessions)));
        transcripts.warm();
        let (tx, rx) = channel();
        App {
            visible: (0..sessions.len()).collect(),
            search: ContentSearch::new(Arc::clone(&transcripts), tx),
            sessions,
            transcripts,
            selected: 0,
            query: String::new(),
            mode: Mode::Search,
            scroll: 0,
            preview: None,
            jump_to_match: false,
            pending_g: false,
            scanning: false,
            errors,
            quit: false,
            open: None,
            shown: None,
            debounce_until: None,
            rx,
            latest: 0,
        }
    }

    pub fn current(&self) -> Option<usize> {
        self.visible.get(self.selected).copied()
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        let query = self.query.trim();
        if query.is_empty() {
            self.visible = (0..self.sessions.len()).collect();
            self.debounce_until = None;
            self.scanning = false;
        } else {
            self.visible = metadata_hits(&self.sessions, query);
            self.debounce_until = Some(Instant::now() + DEBOUNCE);
        }
        self.selected = 0;
        self.jump_to_match = true;
    }

    /// Start a due content scan, take the result of a finished one, and reset
    /// the preview when the selected session changes.
    pub fn tick(&mut self) {
        if self.debounce_until.is_some_and(|at| Instant::now() >= at) {
            self.debounce_until = None;
            self.latest = self.search.start(self.query.trim());
            self.scanning = true;
        }
        while let Ok(result) = self.rx.try_recv() {
            if result.generation != self.latest {
                continue;
            }
            self.scanning = false;
            let keep = self.current();
            let mut merged = metadata_hits(&self.sessions, self.query.trim());
            merged.extend(result.hits);
            merged.sort_unstable();
            merged.dedup();
            self.visible = merged;
            self.selected = keep
                .and_then(|index| self.visible.iter().position(|&i| i == index))
                .unwrap_or(0);
        }
        if self.current() != self.shown {
            self.shown = self.current();
            self.scroll = 0;
            self.jump_to_match = true;
        }
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
        }
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
}
