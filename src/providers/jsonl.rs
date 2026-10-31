use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use rayon::prelude::*;
use serde_json::Value;
use walkdir::WalkDir;

use crate::model::Session;

/// Stop reading a file head after this many lines or this many bytes.
const HEAD_LINES: usize = 120;
const HEAD_BYTES: usize = 4 * 1024 * 1024;

pub struct Found {
    pub path: PathBuf,
    pub modified: DateTime<Local>,
}

/// Build one session from the head of each recent file under `root`.
pub fn discover(
    root: &Path,
    since: DateTime<Local>,
    build: impl Fn(Found, Vec<Value>) -> Session + Sync,
) -> Result<Vec<Result<Session>>> {
    Ok(files(root, since)?
        .into_par_iter()
        .map(|found| {
            let head = head(&found.path)?;
            Ok(build(found, head))
        })
        .collect())
}

/// Every `*.jsonl` file under `root` that changed at or after `since`.
/// A missing root means that the agent is not installed.
fn files(root: &Path, since: DateTime<Local>) -> Result<Vec<Found>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in WalkDir::new(root) {
        let entry = entry.with_context(|| format!("cannot walk {}", root.display()))?;
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        let modified: DateTime<Local> = entry.metadata()?.modified()?.into();
        if modified >= since {
            found.push(Found {
                path: entry.into_path(),
                modified,
            });
        }
    }
    Ok(found)
}

/// Parse the first records of a file. Stops early so that discovery stays fast
/// on multi-megabyte session files.
fn head(path: &Path) -> Result<Vec<Value>> {
    read(path, HEAD_LINES, HEAD_BYTES)
}

/// Parse every record of a file.
pub fn all(path: &Path) -> Result<Vec<Value>> {
    read(path, usize::MAX, usize::MAX)
}

/// Lines that are not valid JSON are skipped: the agent can still write the
/// last line while we read it.
fn read(path: &Path, max_lines: usize, max_bytes: usize) -> Result<Vec<Value>> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut consumed = 0usize;
    let mut line = String::new();
    while records.len() < max_lines && consumed < max_bytes {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if n == 0 {
            break;
        }
        consumed += n;
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            records.push(value);
        }
    }
    Ok(records)
}
