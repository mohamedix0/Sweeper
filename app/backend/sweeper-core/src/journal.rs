//! Append-only JSON-lines write-ahead journal for `Clean` operations.
//!
//! Crash-resilient: intent records are flushed/synced before deletion batches,
//! outcomes recorded after each item/batch, and readers ignore a truncated trailing line.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
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
    /// Length of the file at the last known-good record boundary. A failed write
    /// that tore a line is rolled back to this length before the journal is reused.
    valid_len: u64,
    /// Set when a write failure could not be repaired. Further appends are refused
    /// so a torn record can never be glued onto a later valid record.
    poisoned: bool,
}

/// Test-only failure injection for the post-delete best-effort outcome path.
#[cfg(feature = "test-support")]
mod test_injection {
    use std::cell::Cell;

    /// How an injected post-delete outcome write should fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum WriteFailure {
        /// Fail before writing any bytes.
        BeforeWrite,
        /// Write a prefix of a record, then fail (leaves a torn tail to repair).
        AfterPrefix,
    }

    thread_local! {
        static FAIL_NEXT_BEST_EFFORT_OUTCOME: Cell<Option<WriteFailure>> = const { Cell::new(None) };
        static FAIL_OUTCOME_REPAIR: Cell<bool> = const { Cell::new(false) };
    }

    /// Make the next best-effort outcome write fail (test-support builds only).
    pub fn fail_next_best_effort_outcome(mode: WriteFailure) {
        FAIL_NEXT_BEST_EFFORT_OUTCOME.with(|flag| flag.set(Some(mode)));
    }

    /// Make the next outcome-write repair fail (test-support builds only).
    pub fn fail_outcome_repair_once() {
        FAIL_OUTCOME_REPAIR.with(|flag| flag.set(true));
    }

    pub(super) fn take_fail_best_effort_outcome() -> Option<WriteFailure> {
        FAIL_NEXT_BEST_EFFORT_OUTCOME.with(|flag| flag.take())
    }

    pub(super) fn take_fail_outcome_repair() -> bool {
        FAIL_OUTCOME_REPAIR.with(|flag| flag.take())
    }
}

/// Make the next post-delete best-effort outcome write fail (test-support builds only).
#[cfg(feature = "test-support")]
pub use test_injection::{fail_next_best_effort_outcome, fail_outcome_repair_once, WriteFailure};

impl JournalWriter {
    /// Create/open a journal file in `base_dir\logs` (or `base_dir` itself if it already ends with `logs`/custom test dir)
    /// and rotate old logs, retaining the last [`MAX_RETAINED_LOGS`] files.
    pub fn open_in(base_dir: &Path) -> Result<Self, JournalError> {
        let log_dir = resolve_log_dir(base_dir);
        fs::create_dir_all(&log_dir)?;
        let rotation_failures = rotate_logs_for_new_file(&log_dir, MAX_RETAINED_LOGS)?;
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
            .write(true)
            .open(&path)?;
        Ok(Self {
            path,
            file,
            deferred_errors: rotation_failures,
            valid_len: 0,
            poisoned: false,
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
    /// The file is restored to the last valid record boundary before it can be
    /// reused: `write_all` may append a partial line before reporting an error,
    /// and a later append would otherwise glue a new record onto that fragment.
    pub fn write_outcome_best_effort(&mut self, record: &JournalRecord) {
        if self.poisoned {
            self.deferred_errors
                .push("journal unavailable after a failed repair".to_string());
            return;
        }

        #[cfg(feature = "test-support")]
        let result = match test_injection::take_fail_best_effort_outcome() {
            None => self.write_outcome(record),
            Some(mode) => self.simulate_failed_outcome_write(mode),
        };
        #[cfg(not(feature = "test-support"))]
        let result = self.write_outcome(record);

        if let Err(err) = result {
            let mut message = err.to_string();
            match self.repair_after_failed_append() {
                Ok(()) => {}
                Err(repair_err) => {
                    // The journal may still hold a torn line. Refuse all future
                    // writes so it can never be misinterpreted as valid JSONL.
                    self.poisoned = true;
                    message.push_str(&format!("; journal repair failed: {repair_err}"));
                }
            }
            self.deferred_errors.push(message);
        }
    }

    /// Simulate a best-effort outcome write that fails, optionally after writing
    /// a partial (unterminated) record prefix (test-support builds only).
    #[cfg(feature = "test-support")]
    fn simulate_failed_outcome_write(
        &mut self,
        mode: test_injection::WriteFailure,
    ) -> Result<(), JournalError> {
        use test_injection::WriteFailure;
        match mode {
            WriteFailure::BeforeWrite => Err(JournalError::Io(std::io::Error::other(
                "injected post-delete outcome write failure",
            ))),
            WriteFailure::AfterPrefix => {
                const PARTIAL: &[u8] = b"{\"record\":\"outcome\",\"target_id\":\"partial\"";
                match self
                    .file
                    .seek(SeekFrom::End(0))
                    .and_then(|_| self.file.write_all(PARTIAL))
                {
                    Ok(()) => Err(JournalError::Io(std::io::Error::other(
                        "injected post-delete outcome write failure after prefix",
                    ))),
                    Err(e) => Err(JournalError::Io(e)),
                }
            }
        }
    }

    /// Restore the journal to the last known-good record boundary after a failed
    /// append so the next record is not appended to a torn line.
    fn repair_after_failed_append(&mut self) -> Result<(), JournalError> {
        #[cfg(feature = "test-support")]
        if test_injection::take_fail_outcome_repair() {
            return Err(JournalError::Io(std::io::Error::other(
                "injected journal repair failure",
            )));
        }
        self.file.set_len(self.valid_len)?;
        self.file.flush()?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Take and clear non-fatal journal errors recorded by best-effort writes.
    pub fn take_deferred_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.deferred_errors)
    }

    fn append_record(&mut self, record: &JournalRecord) -> Result<(), JournalError> {
        if self.poisoned {
            return Err(JournalError::Io(std::io::Error::other(
                "journal unavailable after a failed repair",
            )));
        }
        let mut line = serde_json::to_string(record)?;
        line.push('\n');
        // Append semantics via seek-to-end; a plain write handle (rather than
        // append mode) grants the access needed to truncate during repair.
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(line.as_bytes())?;
        self.valid_len = self.valid_len.saturating_add(line.len() as u64);
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
pub fn rotate_logs(log_dir: &Path, max_files: usize) -> Result<Vec<String>, JournalError> {
    if max_files == 0 || !log_dir.exists() {
        return Ok(Vec::new());
    }
    let mut failures = enforce_max_logs(log_dir, max_files.saturating_sub(1).max(1))?;
    failures.extend(enforce_max_logs(log_dir, max_files)?);
    Ok(failures)
}

/// Rotate logs before writing a new activity log file, leaving room for it.
/// Returns any retirement failures so callers can report them.
pub fn rotate_logs_for_new_file(
    log_dir: &Path,
    max_files: usize,
) -> Result<Vec<String>, JournalError> {
    if max_files == 0 || !log_dir.exists() {
        return Ok(Vec::new());
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

/// Enforce at most `max_files` activity logs, returning any retirement failures.
///
/// Retirement is best effort: one locked or malformed log must not abort rotation
/// of the remaining files. Failures are returned so callers can surface them
/// through the journal's deferred-error channel instead of silently discarding
/// them (issue #56 residual).
fn enforce_max_logs(log_dir: &Path, max_files: usize) -> Result<Vec<String>, JournalError> {
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
        return Ok(Vec::new());
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut failures = Vec::new();
    for (_, path) in entries.into_iter().skip(max_files) {
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            #[allow(clippy::disallowed_methods)]
            if let Err(err) = sweeper_winfs::retire_sweeper_log_file(log_dir, name) {
                failures.push(format!("failed to retire log '{name}': {err}"));
            }
        }
    }
    Ok(failures)
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

/// Outcome of writing a scan summary log.
pub struct ScanLogWriteOutcome {
    /// Path of the written `scan-*.jsonl` file.
    pub path: PathBuf,
    /// Best-effort log-retirement warnings. The scan record was still written;
    /// callers should surface these instead of discarding them.
    pub retirement_warnings: Vec<String>,
}

/// Write a completed scan summary log (`scan-*.jsonl`) and rotate activity logs.
///
/// Log retirement is best effort: a non-retirable old log produces a warning in
/// [`ScanLogWriteOutcome::retirement_warnings`] but never prevents the scan record
/// from being written. A failure to write the scan record itself remains fatal.
pub fn write_scan_log(
    base_dir: &Path,
    record: &serde_json::Value,
) -> Result<ScanLogWriteOutcome, JournalError> {
    let log_dir = resolve_log_dir(base_dir);
    fs::create_dir_all(&log_dir)?;
    let retirement_warnings = rotate_logs_for_new_file(&log_dir, MAX_RETAINED_LOGS)?;
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
    Ok(ScanLogWriteOutcome {
        path,
        retirement_warnings,
    })
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}

/// Sum byte counts or item counts with saturating arithmetic. Log aggregates
/// are serialized as decimal strings; a wrap-around would silently emit a wrong
/// value, so every such aggregate funnels through this helper (issue #98).
pub fn saturating_sum_u64(values: impl IntoIterator<Item = u64>) -> u64 {
    values.into_iter().fold(0u64, u64::saturating_add)
}

/// File name of the persistent log-summary index inside a logs directory.
const LOG_SUMMARY_INDEX_NAME: &str = "log-summaries.json";
/// Temporary file used for atomic replacement of the summary index.
const LOG_SUMMARY_INDEX_TMP: &str = "log-summaries.json.tmp";

/// Path of the persistent summary index for `log_dir`.
pub fn log_summary_index_path(log_dir: &Path) -> PathBuf {
    log_dir.join(LOG_SUMMARY_INDEX_NAME)
}

/// Whether `name` is the internal summary index file or its atomic temp file.
/// These are metadata, never activity logs, so listings must skip them.
pub fn is_summary_index_file_name(name: &str) -> bool {
    name == LOG_SUMMARY_INDEX_NAME || name == LOG_SUMMARY_INDEX_TMP
}

/// Compact, fixed-size summary of a single log file.
///
/// Persisted once when a log is created so `list_logs` can build its listing
/// from this metadata (plus filesystem metadata) instead of reading and parsing
/// every JSONL record of every log on each call (issue #98).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogSummary {
    /// Number of intent records (saturating).
    pub intent_count: u64,
    /// Number of outcome records (saturating).
    pub outcome_count: u64,
    /// Total bytes recorded by outcome records (saturating).
    pub total_bytes: u64,
    /// Whether every recorded path is a sandbox path hint.
    pub sandbox_only: bool,
    /// Intents without a matching outcome (saturating).
    pub intent_without_outcome: u64,
}

impl LogSummary {
    /// Build a summary from parsed counts. Each count converts with a
    /// saturating cast so an over-large value can never wrap into a small one.
    pub fn from_counts(
        intent_count: usize,
        outcome_count: usize,
        total_bytes: u64,
        sandbox_only: bool,
        intent_without_outcome: usize,
    ) -> Self {
        Self {
            intent_count: u64::try_from(intent_count).unwrap_or(u64::MAX),
            outcome_count: u64::try_from(outcome_count).unwrap_or(u64::MAX),
            total_bytes,
            sandbox_only,
            intent_without_outcome: u64::try_from(intent_without_outcome).unwrap_or(u64::MAX),
        }
    }
}

/// Persistent map from log id (filename stem) to its [`LogSummary`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogSummaryIndex {
    /// Summaries keyed by log id.
    #[serde(default)]
    pub summaries: BTreeMap<String, LogSummary>,
}

impl LogSummaryIndex {
    /// Load the index from `log_dir`. Missing, unreadable, or malformed files
    /// yield an empty index so a damaged summary can never break log listing.
    pub fn load(log_dir: &Path) -> Self {
        match fs::read(log_summary_index_path(log_dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Look up the summary for `log_id`.
    pub fn get(&self, log_id: &str) -> Option<&LogSummary> {
        self.summaries.get(log_id)
    }

    /// Insert or replace a log's summary.
    pub fn insert(&mut self, log_id: String, summary: LogSummary) {
        self.summaries.insert(log_id, summary);
    }

    /// Drop summaries whose log files no longer exist (e.g. after rotation).
    /// Returns how many entries were removed.
    pub fn remove_stale(&mut self, live_ids: &HashSet<String>) -> usize {
        let before = self.summaries.len();
        self.summaries.retain(|id, _| live_ids.contains(id));
        before.saturating_sub(self.summaries.len())
    }

    /// Persist the index atomically (write temp, then rename over the target).
    pub fn save(&self, log_dir: &Path) -> Result<(), JournalError> {
        fs::create_dir_all(log_dir)?;
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        let tmp = log_dir.join(LOG_SUMMARY_INDEX_TMP);
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, log_summary_index_path(log_dir))?;
        Ok(())
    }
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
    fn best_effort_outcome_prefix_write_failure_repairs_before_reuse() {
        let dir = tempdir().unwrap();
        let mut writer = JournalWriter::open_in(dir.path()).unwrap();
        let path = writer.path().to_path_buf();

        writer
            .write_intent(&intent_record("a", "C:\\Temp\\a", 1, None))
            .unwrap();

        // write_all can append a prefix and then fail; the file now ends in a
        // torn, unterminated fragment that must be rolled back before reuse.
        test_injection::fail_next_best_effort_outcome(WriteFailure::AfterPrefix);
        writer.write_outcome_best_effort(&outcome_record(
            "a",
            "C:\\Temp\\a",
            1,
            "deleted",
            "",
            None,
        ));
        assert!(
            !writer.take_deferred_errors().is_empty(),
            "partial write failure must be deferred"
        );

        // A later intent must append at the valid boundary, producing parseable JSONL.
        writer
            .write_intent(&intent_record("b", "C:\\Temp\\b", 2, None))
            .unwrap();
        drop(writer);

        let recs = read_records(&path).unwrap();
        assert_eq!(
            recs.len(),
            2,
            "journal must remain valid after a torn outcome write was repaired"
        );
        assert!(matches!(&recs[1], JournalRecord::Intent { target_id, .. } if target_id == "b"));
    }

    #[test]
    fn best_effort_outcome_repair_failure_poisons_journal() {
        let dir = tempdir().unwrap();
        let mut writer = JournalWriter::open_in(dir.path()).unwrap();
        writer
            .write_intent(&intent_record("a", "C:\\Temp\\a", 1, None))
            .unwrap();

        test_injection::fail_next_best_effort_outcome(WriteFailure::AfterPrefix);
        test_injection::fail_outcome_repair_once();
        writer.write_outcome_best_effort(&outcome_record(
            "a",
            "C:\\Temp\\a",
            1,
            "deleted",
            "",
            None,
        ));

        let errs = writer.take_deferred_errors();
        assert!(
            errs.iter().any(|e| e.contains("repair failed")),
            "repair failure must be surfaced: {errs:?}"
        );
        assert!(
            writer
                .write_intent(&intent_record("b", "C:\\Temp\\b", 2, None))
                .is_err(),
            "poisoned journal must refuse further appends"
        );
    }

    /// Seed 35 activity logs of the given `prefix`, rotate to 30, and return
    /// how many activity log files survived. Shared by the rotation regressions.
    fn count_logs_after_rotating(prefix: &str) -> usize {
        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        let mut seeded = Vec::new();
        for i in 0..35 {
            let name = format!("{prefix}-{}-{}-0.jsonl", 1_700_000_000u64 + i, i);
            let p = log_dir.join(&name);
            fs::write(&p, format!("{{\"i\":{i}}}\n")).unwrap();
            seeded.push(name);
        }
        rotate_logs(&log_dir, 30).unwrap();
        // Count the exact filenames seeded, independently of the
        // is_activity_log_file predicate that rotation itself relies on. If
        // scan logs were not retired, all 35 seeded files still exist here.
        seeded
            .iter()
            .filter(|name| log_dir.join(name).exists())
            .count()
    }

    #[test]
    fn rotation_keeps_last_n_logs() {
        let count = count_logs_after_rotating("clean");
        assert!(count <= 30, "count={count}");
    }

    #[test]
    fn rotation_retires_scan_logs_as_well_as_clean_logs() {
        // Scan logs must be eligible for retirement, otherwise they grow without bound.
        let count = count_logs_after_rotating("scan");
        assert!(count <= 30, "scan logs were not retired: count={count}");
    }

    /// A log name counted for retention but rejected by the winfs retirement
    /// validator (`clean-bad-shape.jsonl` has no numeric `<secs>-<pid>-<nanos>`
    /// stem) fails to retire. The failure must be surfaced by rotation and
    /// carried into the journal's deferred errors instead of being discarded.
    #[test]
    fn rotation_surfaces_and_defers_retirement_failures() {
        use std::fs::FileTimes;

        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();

        let bad_name = "clean-bad-shape.jsonl";
        let bad = log_dir.join(bad_name);
        fs::write(&bad, b"x\n").unwrap();
        File::options()
            .write(true)
            .open(&bad)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
            .unwrap();
        // 30 valid + 1 malformed log: rotation to 29 must attempt to retire the
        // malformed (oldest) file, whose retirement fails validation.
        for i in 0..30 {
            let name = format!("clean-1700000000-1-{i}.jsonl");
            let path = log_dir.join(&name);
            fs::write(&path, b"x\n").unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(100 + i)))
                .unwrap();
        }

        let failures = rotate_logs_for_new_file(&log_dir, MAX_RETAINED_LOGS).unwrap();
        assert!(
            failures.iter().any(|message| message.contains(bad_name)),
            "retirement failure must be surfaced: {failures:?}"
        );

        // open_in resolves <base>/logs and seeds deferred errors from rotation.
        let mut writer = JournalWriter::open_in(dir.path()).unwrap();
        let deferred = writer.take_deferred_errors();
        assert!(
            deferred.iter().any(|message| message.contains(bad_name)),
            "retirement failure must be deferred: {deferred:?}"
        );
    }

    /// A permanently non-retirable oldest log must not stop a later scan from
    /// writing its record. The retirement failure is a non-fatal warning and the
    /// `scan-*.jsonl` record is still written with the unchanged JSONL format.
    #[test]
    fn scan_record_written_even_when_log_retirement_fails() {
        use std::fs::FileTimes;

        let dir = tempdir().unwrap();
        let log_dir = dir.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();

        let bad_name = "clean-bad-shape.jsonl";
        let bad = log_dir.join(bad_name);
        fs::write(&bad, b"x\n").unwrap();
        File::options()
            .write(true)
            .open(&bad)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(1)))
            .unwrap();
        // 30 valid logs + the oldest malformed one: rotating to keep 29 must
        // attempt (and fail) to retire the malformed file.
        for i in 0..30 {
            let name = format!("clean-1700000000-1-{i}.jsonl");
            let path = log_dir.join(&name);
            fs::write(&path, b"x\n").unwrap();
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(100 + i)))
                .unwrap();
        }

        let record = serde_json::json!({ "kind": "scan", "event": "scan" });
        let outcome = write_scan_log(dir.path(), &record).unwrap();

        assert!(
            outcome
                .retirement_warnings
                .iter()
                .any(|message| message.contains(bad_name)),
            "retirement failure must be surfaced as a warning: {:?}",
            outcome.retirement_warnings
        );
        assert!(
            outcome.path.exists(),
            "scan record must be written despite retirement failure"
        );
        let written = fs::read_to_string(&outcome.path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(written.trim_end()).unwrap();
        assert_eq!(
            parsed, record,
            "on-disk scan record format must be unchanged"
        );
    }

    #[test]
    fn saturating_sum_u64_never_overflows() {
        assert_eq!(saturating_sum_u64([]), 0);
        assert_eq!(saturating_sum_u64([10, 20, 30]), 60);
        assert_eq!(saturating_sum_u64([u64::MAX, 1]), u64::MAX);
        assert_eq!(saturating_sum_u64([u64::MAX, u64::MAX]), u64::MAX);
    }

    #[test]
    fn log_summary_from_counts_saturates_oversized_counts() {
        // Counts arriving from `usize` must saturate, never wrap into a small
        // value (issue #98: consistent saturating arithmetic in serialization).
        let summary = LogSummary::from_counts(usize::MAX, 3, u64::MAX - 1, true, usize::MAX);
        assert_eq!(summary.intent_count, u64::MAX);
        assert_eq!(summary.outcome_count, 3);
        assert_eq!(summary.total_bytes, u64::MAX - 1);
        assert!(summary.sandbox_only);
        assert_eq!(summary.intent_without_outcome, u64::MAX);
    }

    #[test]
    fn log_summary_index_round_trips_and_prunes_stale_entries() {
        let dir = tempdir().unwrap();
        let live = "clean-1700000000-1-1".to_string();
        let stale = "clean-1699999999-1-0".to_string();
        let mut index = LogSummaryIndex::default();
        index.insert(live.clone(), LogSummary::from_counts(2, 2, 300, true, 0));
        index.insert(stale.clone(), LogSummary::from_counts(1, 1, 10, false, 0));
        index.save(dir.path()).unwrap();

        let loaded = LogSummaryIndex::load(dir.path());
        assert_eq!(loaded.summaries.len(), 2);
        assert_eq!(loaded.get(&live).unwrap().total_bytes, 300);

        let mut loaded = loaded;
        let removed = loaded.remove_stale(&[live.clone()].into_iter().collect());
        assert_eq!(removed, 1);
        loaded.save(dir.path()).unwrap();

        let final_index = LogSummaryIndex::load(dir.path());
        assert_eq!(final_index.summaries.len(), 1);
        assert!(final_index.get(&stale).is_none());
        assert_eq!(final_index.get(&live).unwrap().intent_count, 2);
    }

    #[test]
    fn log_summary_index_load_tolerates_missing_and_malformed_files() {
        let dir = tempdir().unwrap();
        assert!(LogSummaryIndex::load(dir.path()).summaries.is_empty());

        let path = log_summary_index_path(dir.path());
        fs::write(&path, b"{not valid json").unwrap();
        assert!(LogSummaryIndex::load(dir.path()).summaries.is_empty());
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
