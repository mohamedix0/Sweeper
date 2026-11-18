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
/// Clean log file prefix.
const LOG_PREFIX: &str = "clean-";
/// Scan log file prefix.
const SCAN_LOG_PREFIX: &str = "scan-";
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
    /// Non-fatal write failures (e.g. an outcome write after a completed deletion).
    deferred_errors: Vec<String>,
}

/// Test-only failure injection for the post-delete best-effort outcome path.
#[cfg(feature = "test-support")]
mod test_injection {
    use std::cell::Cell;

    thread_local! {
        static FAIL_NEXT_BEST_EFFORT_OUTCOME: Cell<bool> = const { Cell::new(false) };
    }

    /// Make the next best-effort outcome write fail (test-support builds only).
    pub fn fail_next_best_effort_outcome() {
        FAIL_NEXT_BEST_EFFORT_OUTCOME.with(|flag| flag.set(true));
    }

    pub(super) fn take_fail_best_effort_outcome() -> bool {
        FAIL_NEXT_BEST_EFFORT_OUTCOME.with(|flag| flag.take())
    }
}

/// Make the next post-delete best-effort outcome write fail (test-support builds only).
#[cfg(feature = "test-support")]
pub use test_injection::fail_next_best_effort_outcome;

impl JournalWriter {
    /// Create/open a journal file in `base_dir\logs` (or `base_dir` itself if it already ends with `logs`/custom test dir)
    /// and rotate old logs, retaining the last [`MAX_RETAINED_LOGS`] files.
    pub fn open_in(base_dir: &Path) -> Result<Self, JournalError> {
        let log_dir = resolve_log_dir(base_dir);
        fs::create_dir_all(&log_dir)?;
        rotate_logs_for_new_file(&log_dir, MAX_RETAINED_LOGS)?;
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
        Ok(Self {
            path,
            file,
            deferred_errors: Vec::new(),
        })
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

    /// Record an outcome after a deletion has already happened.
    ///
    /// A failure here must never discard a deletion that actually took place, so
    /// the error is recorded for the caller/report instead of being propagated.
    pub fn write_outcome_best_effort(&mut self, record: &JournalRecord) {
        #[cfg(feature = "test-support")]
        let injected = test_injection::take_fail_best_effort_outcome();
        #[cfg(not(feature = "test-support"))]
        let injected = false;

        let result = if injected {
            Err(JournalError::Io(std::io::Error::other(
                "injected post-delete outcome write failure",
            )))
        } else {
            self.write_outcome(record)
        };
        if let Err(err) = result {
            self.deferred_errors.push(err.to_string());
        }
    }

    /// Take and clear non-fatal journal errors recorded by best-effort writes.
    pub fn take_deferred_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.deferred_errors)
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

/// Parse journal records from in-memory JSON-lines bytes.
///
/// Only newline-terminated lines are decoded. An unterminated final fragment is
/// ignored regardless of its byte content, so a torn multi-byte UTF-8 tail cannot
/// discard the valid records that preceded it. Malformed terminated lines error.
pub fn parse_records_bytes(data: &[u8]) -> Result<Vec<JournalRecord>, JournalError> {
    let mut records = Vec::new();
    let mut start = 0usize;
    for (idx, byte) in data.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let line = trim_ascii_whitespace_bytes(&data[start..idx]);
        start = idx + 1;
        if line.is_empty() {
            continue;
        }
        let line = std::str::from_utf8(line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        match serde_json::from_str::<JournalRecord>(line) {
            Ok(rec) => records.push(rec),
            Err(e) => return Err(JournalError::Serde(e)),
        }
    }
    // Bytes after the last newline are an unterminated trailing fragment and are
    // deliberately ignored (crash/torn-write recovery).
    Ok(records)
}

/// Read journal records from a file, ignoring a truncated trailing fragment (crash recovery).
pub fn read_records(path: &Path) -> Result<Vec<JournalRecord>, JournalError> {
    let file = File::open(path)?;
    parse_records_from_reader(BufReader::new(file))
}

fn parse_records_from_reader<R: BufRead>(
    mut reader: R,
) -> Result<Vec<JournalRecord>, JournalError> {
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;
    parse_records_bytes(&data)
}

fn trim_ascii_whitespace_bytes(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(|b| b.is_ascii_whitespace()) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(|b| b.is_ascii_whitespace()) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

/// Rotate activity log files in `log_dir`, retaining only the newest `max_files`
/// matching clean/scan JSONL logs.
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

/// Rotate logs before writing a new activity log file, leaving room for it.
pub fn rotate_logs_for_new_file(log_dir: &Path, max_files: usize) -> Result<(), JournalError> {
    if max_files == 0 || !log_dir.exists() {
        return Ok(());
    }
    let keep = max_files.saturating_sub(1).max(1);
    enforce_max_logs(log_dir, keep)
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
        if !is_activity_log_file(&path) {
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
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            #[allow(clippy::disallowed_methods)]
            let _ = sweeper_winfs::retire_sweeper_log_file(log_dir, name);
        }
    }
    Ok(())
}

fn is_activity_log_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    let ext_ok = lower.ends_with(&format!(".{LOG_EXT}"));
    ext_ok && (lower.starts_with(LOG_PREFIX) || lower.starts_with(SCAN_LOG_PREFIX))
}

/// Build a timestamped scan log filename.
pub fn scan_log_filename(ts_secs: u64, pid: u32, nanos: u32) -> String {
    format!("{SCAN_LOG_PREFIX}{ts_secs}-{pid}-{nanos}.{LOG_EXT}")
}

/// Write a completed scan summary log (`scan-*.jsonl`) and rotate activity logs.
pub fn write_scan_log(
    base_dir: &Path,
    record: &serde_json::Value,
) -> Result<PathBuf, JournalError> {
    let log_dir = resolve_log_dir(base_dir);
    fs::create_dir_all(&log_dir)?;
    rotate_logs_for_new_file(&log_dir, MAX_RETAINED_LOGS)?;
    let ts = unix_now_secs();
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let path = log_dir.join(scan_log_filename(ts, pid, nanos));
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    fs::write(&path, line)?;
    Ok(path)
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
    fn read_records_tolerates_truncated_multibyte_tail_and_keeps_prior_records() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("clean-multibyte-tail.jsonl");
        let mut bytes = serde_json::to_string(&intent_record("kept", "C:\\a\\b\\c", 1, None))
            .unwrap()
            .into_bytes();
        bytes.push(b'\n');
        // A torn multi-byte UTF-8 tail (first byte of a 2-byte sequence) with no newline.
        bytes.extend_from_slice(b"{\"record\":\"outcome\",\"path\":\"\xC3");
        fs::write(&path, &bytes).unwrap();

        let recs = read_records(&path).unwrap();
        assert_eq!(recs.len(), 1, "prior record must survive a torn UTF-8 tail");
        assert!(matches!(&recs[0], JournalRecord::Intent { target_id, .. } if target_id == "kept"));

        // The same guarantee holds for in-memory parsing.
        let parsed = parse_records_bytes(&bytes).unwrap();
        assert_eq!(parsed.len(), 1);
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
            .filter(|e| is_activity_log_file(&e.path()))
            .count();
        assert!(count <= 30, "count={count}");
    }

    #[test]
    fn rotation_retires_scan_logs_as_well_as_clean_logs() {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        // Scan logs must be eligible for retirement, otherwise they grow without bound.
        for i in 0..35 {
            let p = log_dir.join(format!("scan-{}-{}-0.jsonl", 1_700_000_000u64 + i, i));
            fs::write(&p, format!("{{\"i\":{i}}}\n")).unwrap();
        }
        rotate_logs(&log_dir, 30).unwrap();
        let count = fs::read_dir(&log_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| is_activity_log_file(&e.path()))
            .count();
        assert!(count <= 30, "scan logs were not retired: count={count}");
    }

    #[test]
    fn parse_records_bytes_handles_valid_malformed_and_binary() {
        let mut bytes = serde_json::to_string(&intent_record("t", "C:\\a", 1, None))
            .unwrap()
            .into_bytes();
        bytes.push(b'\n');
        let recs = parse_records_bytes(&bytes).unwrap();
        assert_eq!(recs.len(), 1);

        let err = parse_records_bytes(b"{bad}\n{}\n");
        assert!(err.is_err());

        let truncated = parse_records_bytes(b"{\"record\":\"intent\"").unwrap();
        assert!(truncated.is_empty());

        // An unterminated final fragment is tolerated regardless of its bytes.
        let binary = parse_records_bytes(&[0xff, 0x00, 0xfe]);
        assert!(binary.unwrap().is_empty());

        // Invalid UTF-8 in a *terminated* line is still an error.
        assert!(parse_records_bytes(&[0xff, b'\n']).is_err());
    }
}
