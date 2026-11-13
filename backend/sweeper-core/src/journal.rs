//! Append-only JSON-lines write-ahead journal for `Clean` operations.
//!
//! Crash-resilient: intent records are flushed/synced before deletion batches,
//! outcomes recorded after each item/batch, and readers ignore a truncated trailing line.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum retained journal/log files.
pub const MAX_RETAINED_LOGS: usize = 30;
/// Log file prefix.
const LOG_PREFIX: &str = "clean-";
/// Log file extension.
const LOG_EXT: &str = "jsonl";

/// Journal errors.
#[derive(Debug, Error)]
pub enum JournalError {
    /// Filesystem/IO error.
    #[error("journal io error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization/deserialization error.
    #[error("journal serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Intent/outcome record types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum JournalRecord {
    /// Intent written and synced before deletion work for an item/batch entry.
    Intent {
        /// Stable scan/validated item id.
        target_id: String,
        /// Item path at planning time.
        path: String,
        /// Estimated size in bytes for this item.
        size: u64,
        /// Unix timestamp seconds.
        timestamp: u64,
        /// Optional batch identifier.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
    /// Outcome written after processing an item.
    Outcome {
        /// Stable scan/validated item id.
        target_id: String,
        /// Item path.
        path: String,
        /// Bytes deleted (0 if skipped/failed).
        bytes: u64,
        /// Outcome status: `deleted` or `Skipped`.
        status: String,
        /// Reason for skip/failure (empty for deleted).
        #[serde(default)]
        reason: String,
        /// Unix timestamp seconds.
        timestamp: u64,
        /// Optional batch identifier.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch_id: Option<String>,
    },
}

/// Append-only journal writer with explicit flush/sync operations.
#[derive(Debug)]
pub struct JournalWriter {
    path: PathBuf,
    file: File,
}

impl JournalWriter {
    /// Create/open a journal file in `base_dir\logs` (or `base_dir` itself if it already ends with `logs`/custom test dir)
    /// and rotate old logs, retaining the last [`MAX_RETAINED_LOGS`] files.
    pub fn open_in(base_dir: &Path) -> Result<Self, JournalError> {
        let log_dir = resolve_log_dir(base_dir);
        fs::create_dir_all(&log_dir)?;
        rotate_logs(&log_dir, MAX_RETAINED_LOGS)?;
        let ts = unix_now_secs();
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos();
        let filename = format!("{LOG_PREFIX}{ts}-{pid}-{nanos}.{LOG_EXT}");
        let path = log_dir.join(filename);
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        Ok(Self { path, file })
    }

    /// Path of the active journal file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append an intent record and flush+sync it to durable storage before deletion work.
    pub fn write_intent(&mut self, record: &JournalRecord) -> Result<(), JournalError> {
        self.append_record(record)?;
        self.flush_and_sync()
    }

    /// Append an outcome record and flush+sync it.
    pub fn write_outcome(&mut self, record: &JournalRecord) -> Result<(), JournalError> {
        self.append_record(record)?;
        self.flush_and_sync()
    }

    fn append_record(&mut self, record: &JournalRecord) -> Result<(), JournalError> {
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Flush and sync the journal file.
    pub fn flush_and_sync(&mut self) -> Result<(), JournalError> {
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(())
    }
}

/// Read journal records from a file, ignoring a truncated trailing incomplete line (crash recovery).
pub fn read_records(path: &Path) -> Result<Vec<JournalRecord>, JournalError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    let mut lines = reader.lines().peekable();
    while let Some(line_res) = lines.next() {
        let line = line_res?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<JournalRecord>(&line) {
            Ok(rec) => records.push(rec),
            Err(e) => {
                // If this is the last line, treat as truncated/crash-mid-write and ignore.
                if lines.peek().is_none() {
                    break;
                }
                return Err(JournalError::Serde(e));
            }
        }
    }
    Ok(records)
}

/// Rotate log files in `log_dir`, retaining only the newest `max_files` matching journal logs.
pub fn rotate_logs(log_dir: &Path, max_files: usize) -> Result<(), JournalError> {
    if max_files == 0 {
        return Ok(());
    }
    if !log_dir.exists() {
        return Ok(());
    }
    enforce_max_logs(log_dir, max_files.saturating_sub(1).max(1))?;
    enforce_max_logs(log_dir, max_files)?;
    Ok(())
}

/// Resolve the directory where journal log files are stored.
pub fn resolve_log_dir(base_dir: &Path) -> PathBuf {
    if base_dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("logs"))
        .unwrap_or(false)
    {
        base_dir.to_path_buf()
    } else {
        base_dir.join("logs")
    }
}

/// Best-effort default base directory for logs (`%LOCALAPPDATA%\Sweeper`).
pub fn default_logs_base_dir() -> PathBuf {
    sweeper_winfs::env_var("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("Sweeper"))
        .unwrap_or_else(|| PathBuf::from(r"C:\Sweeper"))
}

/// Create an intent record with current timestamp.
pub fn intent_record(
    target_id: &str,
    path: &str,
    size: u64,
    batch_id: Option<String>,
) -> JournalRecord {
    JournalRecord::Intent {
        target_id: target_id.to_string(),
        path: path.to_string(),
        size,
        timestamp: unix_now_secs(),
        batch_id,
    }
}

/// Create an outcome record with current timestamp.
pub fn outcome_record(
    target_id: &str,
    path: &str,
    bytes: u64,
    status: &str,
    reason: impl Into<String>,
    batch_id: Option<String>,
) -> JournalRecord {
    JournalRecord::Outcome {
        target_id: target_id.to_string(),
        path: path.to_string(),
        bytes,
        status: status.to_string(),
        reason: reason.into(),
        timestamp: unix_now_secs(),
        batch_id,
    }
}

fn enforce_max_logs(log_dir: &Path, max_files: usize) -> Result<(), JournalError> {
    let mut entries: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !is_journal_log_file(&path) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        entries.push((modified, path));
    }
    if entries.len() <= max_files {
        return Ok(());
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, path) in entries.into_iter().skip(max_files) {
        let _ = sweeper_winfs::retire_log_file(&path);
    }
    Ok(())
}

fn is_journal_log_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.starts_with(LOG_PREFIX) && lower.ends_with(&format!(".{LOG_EXT}"))
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_records_ignores_truncated_trailing_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clean-test.jsonl");
        let mut f = File::create(&path).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&intent_record("t", "C:\\a\\b\\c", 1, None)).unwrap()
        )
        .unwrap();
        // truncated trailing line (no newline / incomplete JSON)
        write!(f, "{{\"record\":\"outcome\",\"target_id\":").unwrap();
        f.flush().unwrap();
        let recs = read_records(&path).unwrap();
        assert_eq!(recs.len(), 1);
    }

    #[test]
    fn rotation_keeps_last_n_logs() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        for i in 0..35 {
            let p = log_dir.join(format!("clean-{}-{}-0.jsonl", 1_700_000_000u64 + i, i));
            fs::write(&p, format!("{{\"i\":{i}}}\n")).unwrap();
        }
        rotate_logs(&log_dir, 30).unwrap();
        let count = fs::read_dir(&log_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| is_journal_log_file(&e.path()))
            .count();
        assert!(count <= 30, "count={count}");
    }
}
