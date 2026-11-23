//! Handle-based `Clean` executor (`remover`) with moment-of-deletion re-validation.
//!
//! Sole module allowed to invoke `sweeper-winfs` deletion primitives (enforced by clippy/gate-a).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::Risk;
use sweeper_winfs::{self, ReparseTagClass, WinfsError};

use crate::journal::{self, JournalError, JournalWriter};
use crate::paths::CanonicalPath;
use crate::policy;

mod case_variant;
mod clean_progress;
mod remover_whole_cache;

use case_variant::reject_case_distinct_sibling;
use clean_progress::{
    emit_item_progress, stop_and_flush, CleanProgressThrottle, FailureProgress, ItemProgress,
};
use remover_whole_cache::{age_check_applies, whole_cache_partial_outcome};

/// Errors from clean/remover operations.
#[derive(Debug, Error)]
pub enum CleanError {
    /// Journal failure.
    #[error("journal error: {0}")]
    Journal(#[from] JournalError),
    /// Filesystem/journal directory setup failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Internal invariant violation.
    #[error("invariant violation: {0}")]
    Invariant(String),
    /// A circuit-breaker stop whose Skipped-outcome journal write also failed.
    ///
    /// Carries the original breaker reason so finalization keeps it as the abort
    /// reason while still recording the journal failure and stopping further appends.
    #[error("journal failure while stopping at circuit breaker ({reason}): {detail}")]
    BreakerJournalFailure {
        /// The circuit-breaker reason that stopped the batch.
        reason: String,
        /// The journal write failure that occurred while recording the skip.
        detail: String,
    },
}

/// Source/authorization context for a validated item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidatedSource {
    /// Built-in allowlisted target root (contents cleaned under this root).
    AllowlistedTarget {
        /// Target id.
        target_id: String,
    },
    /// User-validated custom path root (contents cleaned under this root, root kept).
    CustomPath,
    /// Tier 3 stale project directory (`target/` or `node_modules`) discovered by strict heuristics.
    /// The matched directory itself may be removed after contents are safely cleaned.
    Tier3ProjectDirectory {
        /// Logical target id (e.g. `stale-rust-target`, `stale-node-modules`).
        target_id: String,
    },
    /// Tier 3 recycle-bin scope rooted at a user-accessible recycle-bin directory (contents cleaned, root kept).
    Tier3RecycleBin,
    /// Issue #41 whole-cache scope: the entire, explicitly scoped, fully
    /// regenerable cache directory is removed as one unit. No per-file Minimum
    /// Age filtering and no per-item selection; the root is removed after its
    /// contents. Never used for caches that can contain non-regenerable data.
    WholeCache {
        /// Owning target id (e.g. `uv-cache`, `maven-wrapper-dists`).
        target_id: String,
    },
}

/// A deletion candidate that has passed parse/validation pipeline construction.
///
/// No public constructor: raw strings cannot reach removal APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedItem {
    /// Stable item identifier.
    pub(crate) id: String,
    /// Owning target id or logical group id.
    pub(crate) target_id: String,
    /// Canonical target/custom root path under which deletion is allowed.
    pub(crate) target_root: String,
    /// Canonical item path to delete (file, directory, or link entry).
    pub(crate) path: String,
    /// Estimated logical size in bytes (for accounting/circuit breaker).
    pub(crate) size_bytes: u64,
    /// Last write time as unix seconds (if known).
    pub(crate) mtime_unix_secs: Option<u64>,
    /// Source/authorization kind.
    pub(crate) source: ValidatedSource,
    /// Optional risk label for reporting.
    pub(crate) risk: Option<Risk>,
}

impl ValidatedItem {
    /// Crate-internal constructor for validated construction pipelines and test-support helpers.
    #[cfg(feature = "test-support")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_internal(
        id: String,
        target_id: String,
        target_root: String,
        path: String,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
        source: ValidatedSource,
        risk: Option<Risk>,
    ) -> Self {
        Self {
            id,
            target_id,
            target_root,
            path,
            size_bytes,
            mtime_unix_secs,
            source,
            risk,
        }
    }

    /// Item id.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Target id.
    pub fn target_id(&self) -> &str {
        &self.target_id
    }
    /// Target root path.
    pub fn target_root(&self) -> &str {
        &self.target_root
    }
    /// Item path.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Estimated size bytes.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    /// Last modified time (unix seconds), if known.
    pub fn mtime_unix_secs(&self) -> Option<u64> {
        self.mtime_unix_secs
    }

    /// Construct a validated allowlisted scan item for bridge-mediated clean execution.
    ///
    /// Performs basic lexical/path-policy checks (containment within target root, non-protected,
    /// canonical lexical parsing). Moment-of-deletion handle-based revalidation remains mandatory
    /// inside the remover.
    pub fn from_allowlisted_scan(
        id: impl Into<String>,
        target_id: impl Into<String>,
        target_root: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
    ) -> Result<Self, String> {
        let id = id.into();
        let target_id = target_id.into();
        let target_root = target_root.into();
        let path = path.into();

        let root = CanonicalPath::parse_lexical(&target_root)
            .map_err(|e| format!("invalid target root: {e}"))?;
        let item =
            CanonicalPath::parse_lexical(&path).map_err(|e| format!("invalid item path: {e}"))?;

        if root.segment_count() < 3 {
            return Err("target root depth < 3".to_string());
        }
        if !root.is_self_or_ancestor_of(&item) || item.is_same_path(&root) {
            return Err(
                "item path must be inside target root (and not the root itself)".to_string(),
            );
        }
        if policy::is_subtree_protected(&root)
            || policy::is_ancestor_of_subtree_protected(&root)
            || policy::is_protected_container_root(&root)
        {
            return Err("target root is protected".to_string());
        }
        if policy::is_subtree_protected(&item)
            || policy::is_ancestor_of_subtree_protected(&item)
            || policy::is_protected_container_root(&item)
        {
            return Err("item path is protected".to_string());
        }

        Ok(Self {
            id,
            target_id: target_id.clone(),
            target_root,
            path,
            size_bytes,
            mtime_unix_secs,
            source: ValidatedSource::AllowlistedTarget { target_id },
            risk: None,
        })
    }

    /// Validate a root path with the strict Tier 3 root policy and return its
    /// canonical string form. Shared by the root-style constructors so the
    /// validation/canonicalisation sequence exists in exactly one place.
    fn validated_tier3_root(path: &str, label: &str) -> Result<String, String> {
        let verdict = policy::validate_tier3_target_root(path);
        if !verdict.allowed {
            return Err(format!("{label} not allowed: {}", verdict.reason));
        }
        CanonicalPath::parse_lexical(path)
            .map(|p| p.to_string())
            .map_err(|e| format!("invalid {label}: {e}"))
    }

    /// Build a root-style item whose `target_root` and `path` are the same
    /// canonical directory. All root-style sources carry `Risk::Caution`.
    fn new_root_item(
        id: String,
        target_id: String,
        canon: String,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
        source: ValidatedSource,
    ) -> Self {
        Self {
            id,
            target_id,
            target_root: canon.clone(),
            path: canon,
            size_bytes,
            mtime_unix_secs,
            source,
            risk: Some(Risk::Caution),
        }
    }

    /// Construct a validated custom-path root item (contents-only clean of the root directory).
    ///
    /// The path is validated with `policy::validate_custom_path` and stored as both `target_root`
    /// and `path` so the remover applies contents-only semantics (root directory kept).
    pub fn from_custom_path_root(
        id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let path = path.into();
        let verdict = policy::validate_custom_path(&path);
        if !verdict.allowed {
            return Err(format!("custom path not allowed: {}", verdict.reason));
        }
        let canon = CanonicalPath::parse_lexical(&path)
            .map_err(|e| format!("invalid custom path: {e}"))?
            .to_string();
        Ok(Self::new_root_item(
            id,
            "custom-path".to_string(),
            canon,
            size_bytes,
            None,
            ValidatedSource::CustomPath,
        ))
    }

    /// Construct a validated Tier 3 recycle-bin root item (contents-only; root kept).
    ///
    /// Used for user-scoped recycle-bin cleanup. The root must pass Tier 3 safety validation.
    pub fn from_tier3_recycle_bin_root(
        id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let path = path.into();
        let canon = Self::validated_tier3_root(&path, "recycle bin path")?;
        Ok(Self::new_root_item(
            id,
            "recycle-bin".to_string(),
            canon,
            size_bytes,
            None,
            ValidatedSource::Tier3RecycleBin,
        ))
    }

    /// Construct a validated Tier 3 stale project directory item.
    ///
    /// `path` is the stale `target/` or `node_modules/` directory to remove. `target_root` is the
    /// same directory and may be deleted after its contents are safely cleaned (unlike custom paths).
    pub fn from_tier3_project_directory(
        id: impl Into<String>,
        logical_target_id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
        mtime_unix_secs: Option<u64>,
    ) -> Result<Self, String> {
        let id = id.into();
        let logical_target_id = logical_target_id.into();
        let path = path.into();
        let canon = Self::validated_tier3_root(&path, "stale project directory")?;
        let source = ValidatedSource::Tier3ProjectDirectory {
            target_id: logical_target_id.clone(),
        };
        Ok(Self::new_root_item(
            id,
            logical_target_id,
            canon,
            size_bytes,
            mtime_unix_secs,
            source,
        ))
    }

    /// Construct a validated whole-cache root item (issue #41).
    ///
    /// `path` is the entire cache directory and is also the `target_root`:
    /// the remover cleans its contents and then removes the root itself. The
    /// root must pass the strict Tier 3 root policy (protected paths, link
    /// points, depth >= 3, must exist). Age filtering is disabled for this
    /// source so the cache is never left partially cleared by Minimum Age.
    pub fn from_whole_cache_root(
        id: impl Into<String>,
        target_id: impl Into<String>,
        path: impl Into<String>,
        size_bytes: u64,
    ) -> Result<Self, String> {
        let id = id.into();
        let target_id = target_id.into();
        let path = path.into();
        let canon = Self::validated_tier3_root(&path, "whole-cache root")?;
        let source = ValidatedSource::WholeCache {
            target_id: target_id.clone(),
        };
        Ok(Self::new_root_item(
            id, target_id, canon, size_bytes, None, source,
        ))
    }
}

/// Clean execution options.
#[derive(Debug, Clone)]
pub struct CleanOptions {
    /// Preview estimate used for the 120% circuit breaker.
    pub preview_estimated_bytes: u64,
    /// Minimum age threshold (files newer than this are skipped). `now - mtime < min_age` => skipped.
    pub min_age: Duration,
    /// Current time for age checks.
    pub now: SystemTime,
    /// Base directory for journal logs (defaults to `%LOCALAPPDATA%\Sweeper`).
    pub journal_base_dir: Option<PathBuf>,
}

impl Default for CleanOptions {
    fn default() -> Self {
        Self {
            preview_estimated_bytes: 0,
            min_age: Duration::from_secs(24 * 3600),
            now: SystemTime::now(),
            journal_base_dir: None,
        }
    }
}

/// Per-item clean outcome.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemCleanResult {
    /// Item id.
    pub id: String,
    /// Path.
    pub path: String,
    /// `deleted` or `Skipped`.
    pub status: String,
    /// Reason (empty when deleted).
    pub reason: String,
    /// Bytes accounted as deleted.
    pub bytes_deleted: u64,
}

/// Clean summary/report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CleanReport {
    /// Per-item outcomes.
    pub items: Vec<ItemCleanResult>,
    /// Total bytes deleted/accounted.
    pub bytes_freed: u64,
    /// Number of skipped items.
    pub skipped_count: u64,
    /// Whether execution aborted early due to circuit breaker/invariant escape.
    pub aborted: bool,
    /// Abort reason if any.
    pub abort_reason: Option<String>,
    /// Non-fatal journal failures recorded after a deletion already happened.
    ///
    /// These are surfaced to the user instead of discarding the whole report.
    #[serde(default)]
    pub journal_errors: Vec<String>,
}

/// Progress update emitted during clean execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanProgress {
    /// Target identifier for the current/processed item.
    pub target_id: String,
    /// Number of items completed so far.
    pub completed_items: u64,
    /// Total items in this clean run.
    pub total_items: u64,
    /// Bytes freed so far.
    pub bytes_freed_so_far: u64,
    /// Status label for the last processed item/update.
    pub status: String,
}

/// Execute handle-based clean for validated items.
///
/// Safety-critical: revalidates on opened handles, journals intent before deletion and outcomes after,
/// aborts on 120% overrun or target-root escape, and uses an abort-on-panic guard.
pub fn execute_clean(
    items: Vec<ValidatedItem>,
    opts: CleanOptions,
) -> Result<CleanReport, CleanError> {
    execute_clean_with_progress(items, opts, |_| {})
}

/// Execute handle-based clean for validated items with progress callbacks.
///
/// Safety-critical: revalidates on opened handles, journals intent before deletion and outcomes after,
/// aborts on 120% overrun or target-root escape, and uses an abort-on-panic guard.
// Keep the two failure-flush call sites compact so this method stays under the
// large-method threshold; rustfmt would otherwise expand each context literal.
#[rustfmt::skip]
pub fn execute_clean_with_progress<F>(
    items: Vec<ValidatedItem>,
    opts: CleanOptions,
    mut on_progress: F,
) -> Result<CleanReport, CleanError>
where
    F: FnMut(CleanProgress),
{
    let _panic_guard = AbortOnPanicGuard;
    let mut journal = open_clean_journal(&opts)?;
    let total_items = items.len() as u64;
    let mut report = new_clean_report(items.len());
    let mut deleted_total: u64 = 0;
    let mut completed: u64 = 0;
    let mut progress_throttle = CleanProgressThrottle::new();

    for (index, item) in items.iter().enumerate() {
        if report.aborted {
            if let Err(err) = record_post_abort_skip(
                &mut journal,
                &mut report,
                item,
                &mut completed,
                total_items,
                &mut progress_throttle,
                &mut on_progress,
            ) {
                return Ok(stop_and_flush(report, err, &items[index..], &mut FailureProgress {
                    journal: &mut journal, throttle: &mut progress_throttle, on_progress: &mut on_progress,
                    item, completed, total_items,
                }));
            }
            continue;
        }

        let outcome = match process_item(item, &opts, &mut journal, &mut deleted_total, &mut report)
        {
            Ok(outcome) => outcome,
            Err(err) => {
                // An intent-write failure stops further deletion, but the
                // deletions already completed and all journal errors collected
                // so far must still reach the caller in a partial report.
                return Ok(stop_and_flush(report, err, &items[index..], &mut FailureProgress {
                    journal: &mut journal, throttle: &mut progress_throttle, on_progress: &mut on_progress,
                    item, completed, total_items,
                }));
            }
        };
        let status = apply_process_outcome(item, outcome, &mut report);
        completed = completed.saturating_add(1);
        emit_item_progress(
            &mut progress_throttle,
            &mut on_progress,
            ItemProgress {
                item,
                completed,
                total_items,
                bytes_freed: report.bytes_freed,
                status,
            },
        );
    }

    // Post-delete journal failures are recorded on the report rather than
    // discarding the outcomes of deletions that already took place.
    report.journal_errors = journal.take_deferred_errors();

    Ok(report)
}

/// Open the write-ahead journal used by a Clean run.
fn open_clean_journal(opts: &CleanOptions) -> Result<JournalWriter, CleanError> {
    let base_dir = opts
        .journal_base_dir
        .clone()
        .unwrap_or_else(journal::default_logs_base_dir);
    Ok(JournalWriter::open_in(&base_dir)?)
}

/// Stop a clean batch after a fatal journal failure while preserving the
/// deletions that already completed and every journal error collected so far.
fn stop_after_journal_failure(
    journal: &mut JournalWriter,
    mut report: CleanReport,
    err: CleanError,
    remaining: &[ValidatedItem],
) -> CleanReport {
    let mut errors = journal.take_deferred_errors();
    errors.push(format!("clean stopped: {err}"));
    report.journal_errors = errors;

    // The first stop owns the abort reason. A later failure while recording a
    // post-abort skip must not replace the reason that actually stopped the batch.
    // A breaker stop whose own Skipped-outcome write failed carries its reason
    // on the error, because `apply_process_outcome` never got to set it.
    if !report.aborted {
        report.aborted = true;
        let carried_reason = match &err {
            CleanError::BreakerJournalFailure { reason, .. } => Some(reason.clone()),
            _ => None,
        };
        report.abort_reason = Some(
            carried_reason.unwrap_or_else(|| format!("clean stopped after journal failure: {err}")),
        );
    }

    let reason = report
        .abort_reason
        .clone()
        .unwrap_or_else(|| "aborted".to_string());
    record_missing_statuses(&mut report, remaining, &reason);
    report
}

/// Give every failed or unattempted item a Skipped status row, matching the
/// per-item accounting of the circuit-breaker path. Items that already carry a
/// partial Deleted result are left untouched.
fn record_missing_statuses(report: &mut CleanReport, items: &[ValidatedItem], reason: &str) {
    for item in items {
        if report.items.iter().any(|row| row.id == item.id) {
            continue;
        }
        report.items.push(ItemCleanResult {
            id: item.id.clone(),
            path: item.path.clone(),
            status: "Skipped".to_string(),
            reason: reason.to_string(),
            bytes_deleted: 0,
        });
        report.skipped_count += 1;
    }
}

/// Build a fresh, empty report for a Clean run of `item_count` items.
fn new_clean_report(item_count: usize) -> CleanReport {
    CleanReport {
        items: Vec::with_capacity(item_count),
        bytes_freed: 0,
        skipped_count: 0,
        aborted: false,
        abort_reason: None,
        journal_errors: Vec::new(),
    }
}

/// Record the skip of an item not processed because the batch already aborted,
/// then emit its (throttled) progress update.
#[allow(clippy::too_many_arguments)]
fn record_post_abort_skip<F>(
    journal: &mut JournalWriter,
    report: &mut CleanReport,
    item: &ValidatedItem,
    completed: &mut u64,
    total_items: u64,
    throttle: &mut CleanProgressThrottle,
    on_progress: &mut F,
) -> Result<(), CleanError>
where
    F: FnMut(CleanProgress),
{
    let reason = report
        .abort_reason
        .clone()
        .unwrap_or_else(|| "aborted".to_string());
    record_skip(journal, report, item, &reason)?;
    *completed = completed.saturating_add(1);
    emit_item_progress(
        throttle,
        on_progress,
        ItemProgress {
            item,
            completed: *completed,
            total_items,
            bytes_freed: report.bytes_freed,
            status: "Skipped".to_string(),
        },
    );
    Ok(())
}

/// Apply a per-item processing outcome to the report and return the status
/// label used in its progress update.
fn apply_process_outcome(
    item: &ValidatedItem,
    outcome: ProcessOutcome,
    report: &mut CleanReport,
) -> String {
    match outcome {
        ProcessOutcome::Deleted(bytes) => {
            report.items.push(ItemCleanResult {
                id: item.id.clone(),
                path: item.path.clone(),
                status: "deleted".to_string(),
                reason: String::new(),
                bytes_deleted: bytes,
            });
            report.bytes_freed = report.bytes_freed.saturating_add(bytes);
            "deleted".to_string()
        }
        ProcessOutcome::Skipped(reason, bytes) => {
            report.items.push(ItemCleanResult {
                id: item.id.clone(),
                path: item.path.clone(),
                status: "Skipped".to_string(),
                reason: reason.clone(),
                bytes_deleted: bytes,
            });
            report.bytes_freed = report.bytes_freed.saturating_add(bytes);
            report.skipped_count += 1;
            "Skipped".to_string()
        }
        ProcessOutcome::Aborted(reason, bytes) => {
            report.aborted = true;
            report.abort_reason = Some(reason.clone());
            report.items.push(ItemCleanResult {
                id: item.id.clone(),
                path: item.path.clone(),
                status: "Skipped".to_string(),
                reason: reason.clone(),
                bytes_deleted: bytes,
            });
            report.bytes_freed = report.bytes_freed.saturating_add(bytes);
            report.skipped_count += 1;
            "aborted".to_string()
        }
    }
}

/// Whether the item's source removes the directory root after its contents
/// are cleaned (Tier 3 stale project directories and issue #41 whole caches).
fn source_removes_root_after_contents(source: &ValidatedSource) -> bool {
    matches!(
        source,
        ValidatedSource::Tier3ProjectDirectory { .. } | ValidatedSource::WholeCache { .. }
    )
}

/// Fold completed child bytes into a Tier 3 root outcome that did not delete
/// the root itself. A successful `Deleted` already accounts for them; a
/// `Skipped` or `Aborted` root must not drop the work done on its children.
fn root_outcome_with_child_bytes(outcome: ProcessOutcome, child_bytes: u64) -> ProcessOutcome {
    match outcome {
        ProcessOutcome::Deleted(deleted) => ProcessOutcome::Deleted(deleted),
        ProcessOutcome::Skipped(reason, carried) => {
            ProcessOutcome::Skipped(reason, carried.saturating_add(child_bytes))
        }
        ProcessOutcome::Aborted(reason, carried) => {
            ProcessOutcome::Aborted(reason, carried.saturating_add(child_bytes))
        }
    }
}

enum ProcessOutcome {
    Deleted(u64),
    /// Skipped with a reason and the bytes already deleted by a partial traversal.
    Skipped(String, u64),
    /// Aborted with a reason and the bytes already deleted by a partial traversal.
    Aborted(String, u64),
}

impl ProcessOutcome {
    /// A skip outcome with no completed deletions.
    fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped(reason.into(), 0)
    }

    /// An abort outcome with no completed deletions.
    fn aborted(reason: impl Into<String>) -> Self {
        Self::Aborted(reason.into(), 0)
    }
}

fn process_item(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
) -> Result<ProcessOutcome, CleanError> {
    // Missing target root/custom path: zero deletions, parent untouched.
    let target_root_path = Path::new(&item.target_root);
    if !target_root_path.exists() {
        journal.write_intent(&journal::intent_record(
            &item.id,
            &item.path,
            item.size_bytes,
            None,
        ))?;
        let reason = "Skipped: missing target root (zero deletions; parent untouched)".to_string();
        journal.write_outcome(&journal::outcome_record(
            &item.id, &item.path, 0, "Skipped", &reason, None,
        ))?;
        return Ok(ProcessOutcome::skipped(reason));
    }
    // Revalidate target root/ancestors are not link points at moment of deletion.
    if let Err(reason) = ensure_target_root_safe(&item.target_root) {
        journal.write_intent(&journal::intent_record(
            &item.id,
            &item.path,
            item.size_bytes,
            None,
        ))?;
        journal.write_outcome(&journal::outcome_record(
            &item.id, &item.path, 0, "Skipped", &reason, None,
        ))?;
        if reason.contains("escape")
            || reason.contains("swapped to a junction")
            || (reason.contains("link point") && reason.contains("target root"))
        {
            // Target root compromised -> abort entire clean for safety.
            return Ok(ProcessOutcome::aborted(reason));
        }
        return Ok(ProcessOutcome::skipped(reason));
    }

    let item_path = Path::new(&item.path);
    if !item_path.exists() {
        journal.write_intent(&journal::intent_record(
            &item.id,
            &item.path,
            item.size_bytes,
            None,
        ))?;
        let reason = "Skipped: missing path".to_string();
        journal.write_outcome(&journal::outcome_record(
            &item.id, &item.path, 0, "Skipped", &reason, None,
        ))?;
        return Ok(ProcessOutcome::skipped(reason));
    }

    // Determine if item is the target root itself.
    let is_root_itself = paths_equal_str(&item.path, &item.target_root);
    if is_root_itself {
        if item_path.is_dir() {
            let contents = clean_directory_contents(item, opts, journal, deleted_total, report)?;
            let outcome = contents.outcome;
            if let ProcessOutcome::Deleted(bytes) = outcome {
                // Tier 3 stale project directories and issue #41 whole-cache
                // targets remove the now-cleaned directory root itself.
                if source_removes_root_after_contents(&item.source) {
                    if let Err(reason) = ensure_target_root_safe(&item.target_root) {
                        // The final root safety check aborted after the children
                        // were deleted; keep their bytes instead of reporting zero.
                        return Ok(ProcessOutcome::Aborted(reason, bytes));
                    }
                    let root_empty = sweeper_winfs::read_dir_entries(item_path)
                        .map(|v| v.is_empty())
                        .unwrap_or(false);
                    if root_empty {
                        let result = delete_path_entry_allowing_root(
                            item,
                            &item.path,
                            opts,
                            journal,
                            deleted_total,
                            Some(bytes),
                            contents.root_mutated,
                        );
                        return match result {
                            // Root was not deleted (Skipped/Aborted): carry the
                            // completed child bytes instead of dropping them.
                            Ok(root_outcome) => {
                                Ok(root_outcome_with_child_bytes(root_outcome, bytes))
                            }
                            Err(err) => {
                                // Child deletions already completed even though the
                                // final root-removal intent could not be journaled.
                                // An already-empty root did no work, so only report a
                                // partial deletion when an entry was actually deleted.
                                if contents.deleted_any {
                                    apply_process_outcome(
                                        item,
                                        ProcessOutcome::Deleted(bytes),
                                        report,
                                    );
                                }
                                Err(err)
                            }
                        };
                    }
                    // A whole-cache is all-or-nothing. Leftover entries (for
                    // example a locked file) mean it was NOT fully cleaned, so
                    // report Skipped while still accounting for bytes already
                    // deleted. Tier 3 project directories keep their existing
                    // Deleted outcome.
                    if matches!(item.source, ValidatedSource::WholeCache { .. }) {
                        return Ok(whole_cache_partial_outcome(bytes));
                    }
                    return Ok(ProcessOutcome::Deleted(bytes));
                }
            }
            return Ok(outcome);
        }
        // Do not delete root file unexpectedly.
        journal.write_intent(&journal::intent_record(
            &item.id,
            &item.path,
            item.size_bytes,
            None,
        ))?;
        let reason = "Skipped: target root item".to_string();
        journal.write_outcome(&journal::outcome_record(
            &item.id, &item.path, 0, "Skipped", &reason, None,
        ))?;
        return Ok(ProcessOutcome::skipped(reason));
    }

    delete_path_entry(item, &item.path, opts, journal, deleted_total)
}

struct CleanContentsResult {
    outcome: ProcessOutcome,
    root_mutated: bool,
    /// Whether at least one entry was actually deleted (even a zero-byte file).
    /// Distinguishes "empty root, no work" from a completed partial deletion.
    deleted_any: bool,
}

fn norm_path_key(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

/// Mutable state accumulated while cleaning a directory's contents.
///
/// Keeping this in one place lets the traversal helpers stay small: each one
/// only records bytes/dirs and decides whether the caller must stop early.
struct ContentsState {
    /// Bytes deleted by this contents-clean so far (partial results).
    deleted_bytes: u64,
    /// Whether at least one entry was actually deleted (even a zero-byte file).
    deleted_any: bool,
    /// Directories whose contents changed, used to bypass the emptied-dir age check.
    mutated_dirs: HashSet<String>,
    /// Child directories to remove bottom-up after the file sweep.
    postorder: Vec<PathBuf>,
    /// Normalized key of the contents root.
    root_key: String,
}

impl ContentsState {
    fn new(root: &str) -> Self {
        Self {
            deleted_bytes: 0,
            deleted_any: false,
            mutated_dirs: HashSet::new(),
            postorder: Vec::new(),
            root_key: norm_path_key(Path::new(root)),
        }
    }

    fn root_mutated(&self) -> bool {
        self.mutated_dirs.contains(&self.root_key)
    }

    fn aborted(&self, reason: String) -> CleanContentsResult {
        CleanContentsResult {
            outcome: ProcessOutcome::Aborted(reason, self.deleted_bytes),
            root_mutated: self.root_mutated(),
            deleted_any: self.deleted_any,
        }
    }

    fn skipped(&self, reason: String) -> CleanContentsResult {
        CleanContentsResult {
            outcome: ProcessOutcome::Skipped(reason, self.deleted_bytes),
            root_mutated: self.root_mutated(),
            deleted_any: self.deleted_any,
        }
    }

    fn deleted(&self) -> CleanContentsResult {
        CleanContentsResult {
            outcome: ProcessOutcome::Deleted(self.deleted_bytes),
            root_mutated: self.root_mutated(),
            deleted_any: self.deleted_any,
        }
    }
}

/// A failed target-root recheck during traversal stops the whole clean.
fn check_root(item: &ValidatedItem, state: &ContentsState) -> Option<CleanContentsResult> {
    ensure_target_root_safe(&item.target_root)
        .err()
        .map(|reason| state.aborted(reason))
}

/// Apply the deletions already completed, then propagate a fatal error.
///
/// This is what keeps the two-children partial-result guarantee: a later
/// intent/outcome write failure must not erase earlier successful deletions.
fn fail_after_partial<T>(
    item: &ValidatedItem,
    state: &ContentsState,
    report: &mut CleanReport,
    err: impl Into<CleanError>,
) -> Result<T, CleanError> {
    // Only report a partial deletion when an entry was actually deleted. A
    // zero-byte file is still a deletion, but a first-intent failure is not.
    if state.deleted_any {
        apply_process_outcome(item, ProcessOutcome::Deleted(state.deleted_bytes), report);
    }
    Err(err.into())
}

/// Journal an intent plus a Skipped outcome for one entry (unreadable dir or
/// reparse placeholder). A fatal write preserves already-completed deletions.
fn journal_skip(
    item: &ValidatedItem,
    journal: &mut JournalWriter,
    path: &str,
    size: u64,
    reason: &str,
    state: &ContentsState,
    report: &mut CleanReport,
) -> Result<(), CleanError> {
    if let Err(err) = journal.write_intent(&journal::intent_record(&item.id, path, size, None)) {
        return fail_after_partial(item, state, report, err);
    }
    if let Err(err) = journal.write_outcome(&journal::outcome_record(
        &item.id, path, 0, "Skipped", reason, None,
    )) {
        return fail_after_partial(item, state, report, err);
    }
    Ok(())
}

/// Delete one entry and fold its outcome into the traversal state.
///
/// Returns `Some(result)` only when the caller must stop (abort or the 120%
/// circuit breaker). `mutated_dir` marks the directory whose contents changed.
#[allow(clippy::too_many_arguments)]
fn delete_and_apply(
    item: &ValidatedItem,
    path_str: &str,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
    state: &mut ContentsState,
    mutated_dir: Option<&Path>,
    ignore_emptied_dir_age: bool,
) -> Result<Option<CleanContentsResult>, CleanError> {
    let outcome = match delete_path_entry_with_options(
        item,
        path_str,
        opts,
        journal,
        deleted_total,
        false,
        None,
        ignore_emptied_dir_age,
    ) {
        Ok(outcome) => outcome,
        Err(err) => return fail_after_partial(item, state, report, err),
    };
    match outcome {
        ProcessOutcome::Deleted(bytes) => {
            state.deleted_bytes = state.deleted_bytes.saturating_add(bytes);
            state.deleted_any = true;
            if let Some(dir) = mutated_dir {
                state.mutated_dirs.insert(norm_path_key(dir));
            }
            if circuit_breaker_exceeded(opts.preview_estimated_bytes, *deleted_total) {
                return Ok(Some(
                    state.aborted("aborted: 120% circuit breaker exceeded".to_string()),
                ));
            }
        }
        ProcessOutcome::Aborted(reason, _) => return Ok(Some(state.aborted(reason))),
        ProcessOutcome::Skipped(_, _) => {}
    }
    Ok(None)
}

/// Classify and handle one enumerated entry: unlink link points (never
/// traverse them), skip reparse placeholders, queue real child dirs, delete
/// real files.
#[allow(clippy::too_many_arguments)]
fn apply_entry(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
    state: &mut ContentsState,
    stack: &mut Vec<PathBuf>,
    dir: &Path,
    ent: sweeper_winfs::DirEntry,
) -> Result<Option<CleanContentsResult>, CleanError> {
    match ent.reparse {
        // Unlink only the link entry itself.
        ReparseTagClass::LinkPoint => delete_and_apply(
            item,
            &ent.path.to_string_lossy(),
            opts,
            journal,
            deleted_total,
            report,
            state,
            Some(dir),
            false,
        ),
        // Cloud/unknown reparse points are always Skipped and never traversed.
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            journal_skip(
                item,
                journal,
                &ent.path.to_string_lossy(),
                ent.size,
                "Skipped: reparse point (cloud/unknown)",
                state,
                report,
            )?;
            Ok(None)
        }
        ReparseTagClass::NotReparse => {
            if ent.is_dir {
                stack.push(ent.path);
                Ok(None)
            } else {
                delete_and_apply(
                    item,
                    &ent.path.to_string_lossy(),
                    opts,
                    journal,
                    deleted_total,
                    report,
                    state,
                    Some(dir),
                    false,
                )
            }
        }
    }
}

/// Iteratively sweep files/link entries without following link points.
fn sweep_entries(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
    state: &mut ContentsState,
    stack: &mut Vec<PathBuf>,
) -> Result<Option<CleanContentsResult>, CleanError> {
    while let Some(dir) = stack.pop() {
        if let Some(result) = check_root(item, state) {
            return Ok(Some(result));
        }
        let entries = match sweeper_winfs::read_dir_entries(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                let reason = format!("Skipped: unreadable directory during contents clean: {e}");
                journal_skip(
                    item,
                    journal,
                    &dir.to_string_lossy(),
                    0,
                    &reason,
                    state,
                    report,
                )?;
                return Ok(Some(state.skipped(reason)));
            }
        };
        // Only child directories (not the root itself) are deletion candidates.
        if !paths_equal_str(&dir.to_string_lossy(), &item.path) {
            state.postorder.push(dir.clone());
        }
        for ent in entries {
            if let Some(result) = apply_entry(
                item,
                opts,
                journal,
                deleted_total,
                report,
                state,
                stack,
                &dir,
                ent,
            )? {
                return Ok(Some(result));
            }
        }
    }
    Ok(None)
}

/// Remove now-empty child directories bottom-up, revalidating safety each time.
fn delete_empty_dirs(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
    state: &mut ContentsState,
) -> Result<Option<CleanContentsResult>, CleanError> {
    let postorder = std::mem::take(&mut state.postorder);
    for dir in postorder.into_iter().rev() {
        if let Some(result) = check_root(item, state) {
            return Ok(Some(result));
        }
        // Skip directories that are not empty.
        if !sweeper_winfs::read_dir_entries(&dir)
            .map(|v| v.is_empty())
            .unwrap_or(false)
        {
            continue;
        }
        let ignore_emptied_dir_age = state.mutated_dirs.contains(&norm_path_key(&dir));
        let parent = dir.parent().map(Path::to_path_buf);
        if let Some(result) = delete_and_apply(
            item,
            &dir.to_string_lossy(),
            opts,
            journal,
            deleted_total,
            report,
            state,
            parent.as_deref(),
            ignore_emptied_dir_age,
        )? {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

fn clean_directory_contents(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
) -> Result<CleanContentsResult, CleanError> {
    let mut state = ContentsState::new(&item.path);
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(&item.path)];

    if let Some(result) = sweep_entries(
        item,
        opts,
        journal,
        deleted_total,
        report,
        &mut state,
        &mut stack,
    )? {
        return Ok(result);
    }
    if let Some(result) = delete_empty_dirs(item, opts, journal, deleted_total, report, &mut state)?
    {
        return Ok(result);
    }
    // Represent contents-clean as a deleted summary for the root item context.
    Ok(state.deleted())
}

fn delete_path_entry(
    item: &ValidatedItem,
    path_str: &str,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
) -> Result<ProcessOutcome, CleanError> {
    delete_path_entry_with_options(
        item,
        path_str,
        opts,
        journal,
        deleted_total,
        false,
        None,
        false,
    )
}

fn delete_path_entry_allowing_root(
    item: &ValidatedItem,
    path_str: &str,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    base_bytes_deleted: Option<u64>,
    ignore_emptied_dir_age: bool,
) -> Result<ProcessOutcome, CleanError> {
    delete_path_entry_with_options(
        item,
        path_str,
        opts,
        journal,
        deleted_total,
        true,
        base_bytes_deleted,
        ignore_emptied_dir_age,
    )
}

#[allow(clippy::too_many_arguments)]
fn delete_path_entry_with_options(
    item: &ValidatedItem,
    path_str: &str,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    allow_delete_root: bool,
    base_bytes_deleted: Option<u64>,
    ignore_emptied_dir_age: bool,
) -> Result<ProcessOutcome, CleanError> {
    delete_path_entry_inner(
        item,
        path_str,
        opts,
        journal,
        deleted_total,
        allow_delete_root,
        base_bytes_deleted,
        ignore_emptied_dir_age,
    )
}

#[allow(clippy::too_many_arguments)]
fn delete_path_entry_inner(
    item: &ValidatedItem,
    path_str: &str,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    allow_delete_root: bool,
    base_bytes_deleted: Option<u64>,
    ignore_emptied_dir_age: bool,
) -> Result<ProcessOutcome, CleanError> {
    let path = Path::new(path_str);
    // Intent before each deletion batch/item.
    journal.write_intent(&journal::intent_record(
        &item.id,
        path_str,
        item.size_bytes,
        None,
    ))?;

    if let Err(reason) = ensure_target_root_safe(&item.target_root) {
        journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        return Ok(if reason.contains("escape") || reason.contains("swapped") {
            ProcessOutcome::aborted(reason)
        } else {
            ProcessOutcome::skipped(reason)
        });
    }

    let handle = match open_for_delete(path) {
        Ok(h) => h,
        Err(e) => {
            let reason = if let WinfsError::Win32 { code, .. } = e {
                if sweeper_winfs::is_not_found(code) {
                    "Skipped: missing path".to_string()
                } else if sweeper_winfs::is_lock_like_error(code) {
                    "Skipped: locked".to_string()
                } else {
                    format!("Skipped: cannot open for delete (os error {code})")
                }
            } else {
                format!("Skipped: cannot open for delete: {e}")
            };
            journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(ProcessOutcome::skipped(reason));
        }
    };

    let inspection = match sweeper_winfs::inspect_handle(&handle) {
        Ok(i) => i,
        Err(e) => {
            let reason = format!("Skipped: handle inspection failed: {e}");
            journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(ProcessOutcome::skipped(reason));
        }
    };

    // Moment-of-deletion re-validation on opened handle.
    if let Err(reason) = revalidate_handle(
        item,
        path_str,
        &inspection,
        opts,
        allow_delete_root,
        ignore_emptied_dir_age,
    ) {
        journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        if reason.contains("escape")
            || (reason.contains("target root") && reason.contains("link point"))
        {
            return Ok(ProcessOutcome::aborted(reason));
        }
        return Ok(ProcessOutcome::skipped(reason));
    }

    // Track actual deleted bytes for circuit breaker based on handle-observed file size (or item estimate).
    let bytes_accounted = match get_handle_logical_size(&handle, &inspection) {
        Ok(Some(sz)) => sz,
        Ok(None) => item.size_bytes,
        Err(_) => item.size_bytes,
    };
    if would_exceed_circuit_breaker(
        opts.preview_estimated_bytes,
        *deleted_total,
        bytes_accounted,
    ) {
        let reason = "aborted: 120% circuit breaker exceeded".to_string();
        if let Err(err) = journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        )) {
            // Do not lose the breaker reason when its Skipped-outcome write
            // fails: carry it on the error so finalization records both.
            return Err(CleanError::BreakerJournalFailure {
                reason,
                detail: err.to_string(),
            });
        }
        return Ok(ProcessOutcome::aborted(reason));
    }

    // Cloud/other reparse tags must be skipped (Link Point allowed only as entry deletion).
    match inspection.reparse {
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            let reason = "Skipped: reparse point (cloud/unknown)".to_string();
            journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(ProcessOutcome::skipped(reason));
        }
        ReparseTagClass::LinkPoint | ReparseTagClass::NotReparse => {}
    }

    let delete_result = if inspection_is_directory(&inspection) {
        delete_dir_handle(&handle)
    } else {
        // clear readonly via handle if needed (best-effort)
        let _ = clear_readonly(&handle);
        delete_file_handle(&handle)
    };

    match delete_result {
        Ok(()) => {
            *deleted_total = deleted_total.saturating_add(bytes_accounted);
            let reported = base_bytes_deleted
                .unwrap_or(0)
                .saturating_add(bytes_accounted);
            // The deletion already succeeded; a journal failure must not throw away
            // this outcome or abort the remaining batch.
            journal.write_outcome_best_effort(&journal::outcome_record(
                &item.id, path_str, reported, "deleted", "", None,
            ));
            Ok(ProcessOutcome::Deleted(reported))
        }
        Err(e) => {
            let reason = match e {
                WinfsError::Win32 { code, .. } if sweeper_winfs::is_not_found(code) => {
                    "Skipped: missing path".to_string()
                }
                WinfsError::Win32 { code, .. } if sweeper_winfs::is_lock_like_error(code) => {
                    "Skipped: locked".to_string()
                }
                other => format!("Skipped: delete failed: {other}"),
            };
            journal.write_outcome_best_effort(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ));
            Ok(ProcessOutcome::skipped(reason))
        }
    }
}

fn revalidate_handle(
    item: &ValidatedItem,
    requested_path: &str,
    inspection: &sweeper_winfs::HandleInspection,
    opts: &CleanOptions,
    allow_delete_root: bool,
    ignore_emptied_dir_age: bool,
) -> Result<(), String> {
    let final_path = strip_extended_prefix(&inspection.final_path);
    let final_canon = CanonicalPath::parse_lexical(&final_path)
        .map_err(|e| format!("Skipped: final path invalid: {e}"))?;
    let target_root = CanonicalPath::parse_lexical(&item.target_root)
        .map_err(|e| format!("Skipped: target root invalid: {e}"))?;

    reject_confirmed_path_redirect(&final_canon, requested_path, &target_root)?;

    // 1) Authorized source (allowlisted target, validated custom path, or opt-in Tier 3 conditional target).
    match &item.source {
        ValidatedSource::AllowlistedTarget { .. }
        | ValidatedSource::CustomPath
        | ValidatedSource::Tier3ProjectDirectory { .. }
        | ValidatedSource::Tier3RecycleBin
        | ValidatedSource::WholeCache { .. } => {}
    }

    // 2) Protected Path exclusion (Container & Subtree via canonical handle identity).
    if policy::is_subtree_protected(&final_canon)
        || policy::is_ancestor_of_subtree_protected(&final_canon)
    {
        return Err("Skipped: protected path".to_string());
    }
    if policy::is_protected_container_root(&final_canon) {
        return Err("Skipped: protected container root".to_string());
    }

    // 3) Final path containment within Target root (and target root/ancestor itself is not a Link Point checked separately).
    if !target_root.is_self_or_ancestor_of(&final_canon) {
        return Err("aborted: handle escaped target root".to_string());
    }
    if final_canon.is_same_path(&target_root) {
        if !(allow_delete_root && source_removes_root_after_contents(&item.source)) {
            // Must not delete the target/custom/recycle-bin root itself.
            return Err("Skipped: target root must not be deleted".to_string());
        }
    } else if path_has_link_point_ancestor(&final_canon, &target_root, true)
        .map_err(|e| format!("Skipped: ancestor reparse check failed: {e}"))?
    {
        return Err("aborted: target root/ancestor link point detected".to_string());
    }
    // For root-deletion case, still ensure ancestors up to (but excluding) root were not link points in ensure_target_root_safe;
    // and root itself must not be a link point (checked below/elsewhere).
    if final_canon.is_same_path(&target_root)
        && path_has_link_point_ancestor(&target_root, &root_parent_or_self(&target_root), false)
            .map_err(|e| format!("Skipped: ancestor reparse check failed: {e}"))?
    {
        return Err("aborted: target root/ancestor link point detected".to_string());
    }
    // (ancestor link-point checks handled above)

    // 4) Reparse tag checks.
    match inspection.reparse {
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            return Err("Skipped: reparse point (cloud/unknown)".to_string());
        }
        ReparseTagClass::LinkPoint => {
            // Allowed to unlink only the link entry itself (no traversal). Ensure it is inside target.
            // (Actual traversal prevention handled by never recursing into link entries.)
        }
        ReparseTagClass::NotReparse => {}
    }

    // 5) Minimum Age check (exact boundary and future mtime treated as recent / Skipped).
    // Whole-cache sources are exempt; otherwise only directories emptied this
    // run are exempt, and the opened handle must still be a real directory.
    if age_check_applies(item, inspection, ignore_emptied_dir_age) {
        let mtime_secs = filetime_to_unix_secs(inspection.last_write_time);
        let mtime = UNIX_EPOCH + Duration::from_secs(mtime_secs);
        if mtime > opts.now {
            return Err("Skipped: future mtime (treated as recent)".to_string());
        }
        if let Ok(age) = opts.now.duration_since(mtime) {
            if age < opts.min_age {
                return Err("Skipped: newer than minimum age".to_string());
            }
        } else {
            return Err("Skipped: future mtime (treated as recent)".to_string());
        }
    }

    Ok(())
}

/// Reject a confirmed path whose handle-authoritative resolution no longer agrees
/// with it (D1 / ADR 0003).
///
/// `inspection.final_path` resolves every component of the path used for the
/// open, while `FILE_FLAG_OPEN_REPARSE_POINT` protects only the final component.
/// An interior junction can therefore be traversed during the open and removed
/// before this check, so an ancestor-only re-check would miss it: the handle
/// then targets an unselected sibling that is still inside the target root.
/// Comparing the two canonical paths (case-insensitive) closes that window. The
/// ancestor re-check is defence in depth for a junction inserted after the open.
/// The final component is excluded from the ancestor check so an item that is
/// itself a Link Point can still be unlinked.
fn reject_confirmed_path_redirect(
    final_canon: &CanonicalPath,
    requested_path: &str,
    target_root: &CanonicalPath,
) -> Result<(), String> {
    let requested_canon = CanonicalPath::parse_lexical(requested_path)
        .map_err(|e| format!("Skipped: requested path invalid: {e}"))?;
    if !final_canon.is_same_path(&requested_canon) {
        return Err("aborted: escape via handle final path divergence from confirmed path (link point traversed under target root)".to_string());
    }
    reject_case_distinct_sibling(final_canon, &requested_canon)?;
    if path_has_link_point_ancestor(&requested_canon, target_root, true)
        .map_err(|e| format!("Skipped: requested-path ancestor check failed: {e}"))?
    {
        return Err(
            "aborted: requested-path ancestor link point detected under target root".to_string(),
        );
    }
    Ok(())
}

fn ensure_target_root_safe(target_root: &str) -> Result<(), String> {
    let root = CanonicalPath::parse_lexical(target_root)
        .map_err(|e| format!("Skipped: invalid target root: {e}"))?;
    let handle = sweeper_winfs::open_for_inspection(Path::new(target_root))
        .map_err(|e| format!("Skipped: cannot inspect target root: {e}"))?;
    let inspection = sweeper_winfs::inspect_handle(&handle)
        .map_err(|e| format!("Skipped: target root inspection failed: {e}"))?;
    let final_path = strip_extended_prefix(&inspection.final_path);
    let final_root = CanonicalPath::parse_lexical(&final_path)
        .map_err(|e| format!("Skipped: target root final path invalid: {e}"))?;
    if !final_root.is_same_path(&root) {
        return Err("aborted: target root escaped or changed".to_string());
    }
    if matches!(inspection.reparse, ReparseTagClass::LinkPoint) {
        return Err("aborted: target root is a link point".to_string());
    }
    // Also reject if any ancestor is link point.
    if path_has_link_point_ancestor(&root, &root_parent_or_self(&root), false)
        .map_err(|e| format!("Skipped: target root ancestor check failed: {e}"))?
    {
        return Err("aborted: target root ancestor is a link point".to_string());
    }
    Ok(())
}

fn root_parent_or_self(root: &CanonicalPath) -> CanonicalPath {
    // For ancestor traversal helper boundary we can use drive root as boundary; easiest to compare against self in loop logic.
    // Return root itself; helper treats equality boundary appropriately.
    root.clone()
}

/// Maximum number of directory-level ancestor chains kept in the cache.
/// Bounding it keeps memory flat even for a Clean run over a very large tree.
const ANCESTOR_CHAIN_CACHE_CAPACITY: usize = 512;

/// Precomputed canonical ancestor paths for one directory level.
///
/// This caches only immutable path strings. It never stores a safety verdict:
/// every item still opens and inspects each ancestor handle at the moment of
/// deletion, so ADR 0003 handle-based revalidation and TOCTOU protection are
/// unchanged. The optimization is avoiding repeated path construction for
/// every item in the same directory ("cached per directory level").
#[derive(Debug, Clone)]
struct AncestorChain {
    /// Canonical paths from the drive root down to and including this level.
    paths: Vec<CanonicalPath>,
}

/// Lazily yield canonical prefixes from the drive root down to and including
/// `directory_level`, skipping segments that fail lexical parsing.
fn ancestor_prefixes(directory_level: &CanonicalPath) -> impl Iterator<Item = CanonicalPath> + '_ {
    let drive_root = PathBuf::from(format!(r"{}\", directory_level.drive_prefix()));
    directory_level
        .segments()
        .scan(drive_root, |current, segment| {
            current.push(segment);
            Some(current.clone())
        })
        .filter_map(|prefix| CanonicalPath::parse_lexical(&prefix.to_string_lossy()).ok())
}

impl AncestorChain {
    fn build(directory_level: &CanonicalPath) -> Self {
        Self {
            paths: ancestor_prefixes(directory_level).collect(),
        }
    }
}

/// Canonical prefixes from the drive root down to `directory_level`, generated
/// lazily. `None` means the caller should stop inspecting: either the prefix
/// did not parse or it does not exist on disk.
fn inspect_ancestor_prefix(prefix: &CanonicalPath) -> Result<Option<bool>, String> {
    #[cfg(test)]
    ANCESTOR_PREFIX_CHECKS.with(|checks| checks.set(checks.get() + 1));
    match sweeper_winfs::open_for_inspection(Path::new(prefix.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
    {
        Ok(ReparseTagClass::LinkPoint) => Ok(Some(true)),
        Ok(_) => Ok(Some(false)),
        Err(WinfsError::Win32 { code, .. }) if sweeper_winfs::is_not_found(code) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Inspect one prefix at a time for an over-budget chain. Each canonical path
/// is dropped before the next is generated, so transient memory stays O(depth)
/// rather than O(depth²) even though retained cache cost stays zero.
fn inspect_deep_ancestors(directory_level: &CanonicalPath) -> Result<bool, String> {
    for prefix in ancestor_prefixes(directory_level) {
        match inspect_ancestor_prefix(&prefix)? {
            Some(true) => return Ok(true),
            Some(false) => {}
            None => return Ok(false),
        }
    }
    Ok(false)
}

/// Retained-cost budget, in segment slots across all cached chains. A chain of
/// depth `d` retains `1 + 2 + ... + d` segment slots because every canonical
/// prefix owns its segment strings. Bounding the *complete* cost (not just the
/// entry count) stops a handful of very deep directories from exhausting memory.
const ANCESTOR_CHAIN_RETAINED_COST_BUDGET: usize = 16 * 1024;

/// Segment slots retained by one chain of the given depth: every canonical
/// prefix owns its segment strings, so the cost grows quadratically.
fn ancestor_chain_cost(segment_count: usize) -> usize {
    segment_count.saturating_mul(segment_count.saturating_add(1)) / 2
}

thread_local! {
    static ANCESTOR_CHAIN_CACHE: RefCell<HashMap<String, AncestorChain>> =
        RefCell::new(HashMap::new());
    static ANCESTOR_CHAIN_RETAINED_COST: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    static ANCESTOR_CHAIN_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    static ANCESTOR_PREFIX_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `f` with the ancestor chain for `directory_level`, building it at most
/// once per level. `directory_level` is always a directory: for a per-item
/// check it is the item's parent directory, so siblings share one entry.
/// Entries are immutable path strings only, never safety verdicts. Expensive
/// deep chains are built for this call but not retained; ordinary shallow
/// directories stay cached within both the entry cap and the cost budget.
fn with_ancestor_chain<R, F>(directory_level: &CanonicalPath, f: F) -> R
where
    F: FnOnce(&AncestorChain) -> R,
{
    let cost = ancestor_chain_cost(directory_level.segment_count());
    if cost > ANCESTOR_CHAIN_RETAINED_COST_BUDGET {
        // A deep chain would dominate the budget on its own. Still build and
        // inspect every prefix for this item, but never retain it in the cache.
        let chain = AncestorChain::build(directory_level);
        #[cfg(test)]
        ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(builds.get() + 1));
        return f(&chain);
    }
    ANCESTOR_CHAIN_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(directory_level.as_str()) {
            let retained = ANCESTOR_CHAIN_RETAINED_COST.get();
            if cache.len() >= ANCESTOR_CHAIN_CACHE_CAPACITY
                || retained.saturating_add(cost) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET
            {
                cache.clear();
                ANCESTOR_CHAIN_RETAINED_COST.set(0);
            }
            let chain = AncestorChain::build(directory_level);
            cache.insert(directory_level.as_str().to_string(), chain);
            ANCESTOR_CHAIN_RETAINED_COST.set(ANCESTOR_CHAIN_RETAINED_COST.get() + cost);
            #[cfg(test)]
            ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(builds.get() + 1));
        }
        let chain = cache
            .get(directory_level.as_str())
            .expect("ancestor chain was just ensured to exist");
        f(chain)
    })
}

#[cfg(test)]
fn ancestor_chain_cache_len() -> usize {
    ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow().len())
}

#[cfg(test)]
fn ancestor_chain_retained_cost() -> usize {
    ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.get())
}

#[cfg(test)]
fn ancestor_chain_build_count() -> usize {
    ANCESTOR_CHAIN_BUILDS.with(|builds| builds.get())
}

fn path_has_link_point_ancestor(
    path: &CanonicalPath,
    _boundary_root: &CanonicalPath,
    exclude_final_path: bool,
) -> Result<bool, String> {
    // A specific boundary root is considered by the callers; the full chain
    // from the drive root is always inspected so no ancestor check is dropped.
    // The chain is keyed by directory level: per-item checks use the item's
    // parent directory (so siblings share one cached chain), while a check
    // that must inspect `path` itself is already a directory level.
    let directory_level = if exclude_final_path {
        path.parent().unwrap_or_else(|| path.clone())
    } else {
        path.clone()
    };
    if ancestor_chain_cost(directory_level.segment_count()) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET {
        // Stream expensive chains instead of materialising every prefix; the
        // ordinary cached path below is unchanged for affordable directories.
        #[cfg(test)]
        ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(builds.get() + 1));
        return inspect_deep_ancestors(&directory_level);
    }
    with_ancestor_chain(&directory_level, |chain| {
        for current in &chain.paths {
            // `current` is always an ancestor directory, never the item itself:
            // the chain stops at the parent level for per-item checks. Every
            // ancestor is still opened and inspected here at deletion time.
            match inspect_ancestor_prefix(current)? {
                Some(true) => return Ok(true),
                Some(false) => {}
                None => return Ok(false),
            }
        }
        Ok(false)
    })
}

fn get_handle_logical_size(
    _handle: &sweeper_winfs::SafeHandle,
    inspection: &sweeper_winfs::HandleInspection,
) -> Result<Option<u64>, WinfsError> {
    if inspection_is_directory(inspection) {
        Ok(Some(0))
    } else {
        Ok(Some(inspection.size))
    }
}

fn inspection_is_directory(inspection: &sweeper_winfs::HandleInspection) -> bool {
    (inspection.attributes & 0x10) != 0 // FILE_ATTRIBUTE_DIRECTORY
}

fn circuit_breaker_exceeded(preview_estimated_bytes: u64, deleted_total: u64) -> bool {
    if preview_estimated_bytes == 0 {
        return deleted_total > 0; // any deletion exceeds 120% of 0
    }
    deleted_total > preview_estimated_bytes.saturating_mul(120) / 100
}

fn would_exceed_circuit_breaker(
    preview_estimated_bytes: u64,
    deleted_total: u64,
    next_bytes: u64,
) -> bool {
    circuit_breaker_exceeded(
        preview_estimated_bytes,
        deleted_total.saturating_add(next_bytes),
    )
}

fn strip_extended_prefix(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

fn filetime_to_unix_secs(ft: (u32, u32)) -> u64 {
    let v = ((ft.0 as u64) << 32) | (ft.1 as u64);
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    if v <= EPOCH_DIFF_100NS {
        0
    } else {
        (v - EPOCH_DIFF_100NS) / 10_000_000
    }
}

fn paths_equal_str(a: &str, b: &str) -> bool {
    match (
        CanonicalPath::parse_lexical(a),
        CanonicalPath::parse_lexical(b),
    ) {
        (Ok(x), Ok(y)) => x.is_same_path(&y),
        _ => a.eq_ignore_ascii_case(b),
    }
}

fn record_skip(
    journal: &mut JournalWriter,
    report: &mut CleanReport,
    item: &ValidatedItem,
    reason: &str,
) -> Result<(), CleanError> {
    journal.write_intent(&journal::intent_record(
        &item.id,
        &item.path,
        item.size_bytes,
        None,
    ))?;
    journal.write_outcome(&journal::outcome_record(
        &item.id, &item.path, 0, "Skipped", reason, None,
    ))?;
    report.items.push(ItemCleanResult {
        id: item.id.clone(),
        path: item.path.clone(),
        status: "Skipped".to_string(),
        reason: reason.to_string(),
        bytes_deleted: 0,
    });
    report.skipped_count += 1;
    Ok(())
}

/// Abort-on-panic guard for critical remover execution.
struct AbortOnPanicGuard;

impl Drop for AbortOnPanicGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            std::process::abort();
        }
    }
}

// --- sweeper-winfs deletion wrappers (sole allowed callsites in production) ---

#[allow(clippy::disallowed_methods)]
fn open_for_delete(path: &Path) -> Result<sweeper_winfs::SafeHandle, WinfsError> {
    sweeper_winfs::open_for_delete(path)
}

#[allow(clippy::disallowed_methods)]
fn delete_file_handle(handle: &sweeper_winfs::SafeHandle) -> Result<(), WinfsError> {
    sweeper_winfs::set_file_disposition_delete(handle)
}

#[allow(clippy::disallowed_methods)]
fn delete_dir_handle(handle: &sweeper_winfs::SafeHandle) -> Result<(), WinfsError> {
    sweeper_winfs::set_file_disposition_delete(handle)
}

#[allow(clippy::disallowed_methods)]
fn clear_readonly(handle: &sweeper_winfs::SafeHandle) -> Result<(), WinfsError> {
    sweeper_winfs::clear_readonly_attribute(handle)
}

/// Controlled constructor for integration tests and trusted pipelines within this crate's test surface.
///
/// Kept outside general app APIs; `ValidatedItem` fields remain private and removal APIs accept only `ValidatedItem`.
#[cfg(feature = "test-support")]
pub fn validated_item_for_tests(
    id: impl Into<String>,
    target_id: impl Into<String>,
    target_root: impl Into<String>,
    path: impl Into<String>,
    size_bytes: u64,
    mtime_unix_secs: Option<u64>,
    source: ValidatedSource,
) -> ValidatedItem {
    ValidatedItem::new_internal(
        id.into(),
        target_id.into(),
        target_root.into(),
        path.into(),
        size_bytes,
        mtime_unix_secs,
        source,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn handle_final_path_divergence_from_confirmed_path_aborts() {
        // Residual D1 TOCTOU: the handle was opened while `sub` was a junction
        // to `keep`; the junction was swapped back to an ordinary directory
        // before revalidation. The ancestor re-check alone now sees no Link
        // Point, but the handle still targets the unselected `keep/victim.txt`.
        let target_root = r"C:\Temp\d1";
        let item = validated_item_for_tests(
            "d1-race",
            "test-target",
            target_root,
            r"C:\Temp\d1\sub\victim.txt",
            14,
            Some(0),
            ValidatedSource::CustomPath,
        );
        let inspection = sweeper_winfs::HandleInspection {
            final_path: r"C:\Temp\d1\keep\victim.txt".to_string(),
            identity: sweeper_winfs::FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            attributes: 0x80, // FILE_ATTRIBUTE_NORMAL
            size: 14,
            reparse: ReparseTagClass::NotReparse,
            reparse_tag: 0,
            last_write_time: (0, 0),
        };
        let opts = CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(0),
            now: SystemTime::now(),
            journal_base_dir: None,
        };

        let reason = revalidate_handle(&item, item.path(), &inspection, &opts, false, false)
            .expect_err("divergent handle final path must abort revalidation");
        assert!(
            reason.contains("escape"),
            "reason must classify as an abort: {reason}"
        );
    }

    #[test]
    fn ancestor_chain_cache_precomputes_levels_and_stays_bounded() {
        let dir = CanonicalPath::parse_lexical(r"C:\Temp\a\b\c").unwrap();
        with_ancestor_chain(&dir, |chain| {
            let paths: Vec<&str> = chain.paths.iter().map(|p| p.as_str()).collect();
            assert_eq!(
                paths,
                vec![r"C:\Temp", r"C:\Temp\a", r"C:\Temp\a\b", r"C:\Temp\a\b\c"]
            );
        });

        // The cache is a bounded optimization: it may not grow without limit.
        for i in 0..(ANCESTOR_CHAIN_CACHE_CAPACITY * 3) {
            let p = CanonicalPath::parse_lexical(&format!(r"C:\Temp\level{i}")).unwrap();
            with_ancestor_chain(&p, |_| ());
        }
        assert!(ancestor_chain_cache_len() <= ANCESTOR_CHAIN_CACHE_CAPACITY);
    }

    #[test]
    fn ancestor_chain_cache_shares_siblings_and_revalidates_handles_after_tamper() {
        use sweeper_testkit::{create_junction, Sandbox};

        // Reset this thread's cache so the build count is unambiguous.
        ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow_mut().clear());
        ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.set(0));
        ANCESTOR_CHAIN_BUILDS.with(|builds| builds.set(0));

        let sb = Sandbox::new();
        let parent = sb.create_dir("target/level0").unwrap();
        let level = parent.join("level1");
        let level_canon = CanonicalPath::parse_lexical(&level.to_string_lossy()).unwrap();
        let mirror = sb.create_dir("mirror").unwrap();

        // Per-item checks exclude the final path, so all sibling files map to
        // the same parent directory level and must share a single cache entry.
        for i in 0..64 {
            let sibling = CanonicalPath::parse_lexical(
                &level.join(format!("sibling{i}.bin")).to_string_lossy(),
            )
            .unwrap();
            assert!(!path_has_link_point_ancestor(&sibling, &level_canon, true).unwrap());
        }
        assert_eq!(
            ancestor_chain_cache_len(),
            1,
            "siblings must share one entry"
        );
        assert_eq!(
            ancestor_chain_build_count(),
            1,
            "chain built once per level"
        );

        // Tamper after the cache is warm: the level becomes a Link Point. The
        // cached strings are unchanged, but each call opens and inspects the
        // ancestor handle again, so the Link Point is detected with no rebuild.
        create_junction(&level, &mirror).unwrap();
        assert!(path_has_link_point_ancestor(&level_canon, &level_canon, false).unwrap());
        assert_eq!(
            ancestor_chain_build_count(),
            1,
            "warm cache must be reused, not trusted as a verdict"
        );
    }

    #[test]
    fn ancestor_chain_cache_bounds_complete_cost_of_deep_chains() {
        ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow_mut().clear());
        ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.set(0));

        let mut deep = String::from(r"C:\Temp");
        for i in 0..200 {
            deep.push_str(&format!(r"\level{i}"));
        }
        let deep_canon = CanonicalPath::parse_lexical(&deep).unwrap();
        assert!(
            ancestor_chain_cost(deep_canon.segment_count()) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET,
            "fixture must exceed the retained-cost budget"
        );

        for _ in 0..8 {
            with_ancestor_chain(&deep_canon, |chain| {
                assert_eq!(chain.paths.len(), deep_canon.segment_count());
            });
        }
        assert_eq!(
            ancestor_chain_cache_len(),
            0,
            "deep chains must not be retained"
        );
        assert_eq!(ancestor_chain_retained_cost(), 0);

        // Ordinary shallow directories remain cacheable within the budget.
        let shallow = CanonicalPath::parse_lexical(r"C:\Temp\a\b\c").unwrap();
        with_ancestor_chain(&shallow, |_| ());
        assert_eq!(ancestor_chain_cache_len(), 1);
        assert!(ancestor_chain_retained_cost() <= ANCESTOR_CHAIN_RETAINED_COST_BUDGET);
    }

    #[test]
    fn deep_ancestor_chain_is_streamed_without_materialising_prefixes() {
        use sweeper_testkit::Sandbox;

        ANCESTOR_CHAIN_CACHE.with(|cache| cache.borrow_mut().clear());
        ANCESTOR_CHAIN_RETAINED_COST.with(|cost| cost.set(0));
        ANCESTOR_PREFIX_CHECKS.with(|checks| checks.set(0));

        let sb = Sandbox::new();
        // 185 real levels exceed the cached cost budget (depth >= 181).
        let deep = sweeper_testkit::create_deep_tree(sb.root(), 185).unwrap();
        let deep_canon = CanonicalPath::from_existing_path(&deep.to_string_lossy())
            .unwrap()
            .into_canonical();
        assert!(
            ancestor_chain_cost(deep_canon.segment_count()) > ANCESTOR_CHAIN_RETAINED_COST_BUDGET
        );

        assert!(!path_has_link_point_ancestor(&deep_canon, &deep_canon, false).unwrap());
        assert_eq!(
            ANCESTOR_PREFIX_CHECKS.with(|checks| checks.get()),
            deep_canon.segment_count(),
            "every prefix must be inspected one at a time"
        );
        assert_eq!(ancestor_chain_cache_len(), 0);
        assert_eq!(ancestor_chain_retained_cost(), 0);
    }

    #[test]
    fn journal_failure_after_breaker_keeps_first_abort_reason() {
        let dir = tempdir().unwrap();
        let mut journal = JournalWriter::open_in(dir.path()).unwrap();
        let mut report = new_clean_report(2);
        report.aborted = true;
        report.abort_reason = Some("aborted: 120% circuit breaker exceeded".to_string());

        let report = stop_after_journal_failure(
            &mut journal,
            report,
            CleanError::Invariant("later skip write failed".to_string()),
            &[],
        );

        assert_eq!(
            report.abort_reason.as_deref(),
            Some("aborted: 120% circuit breaker exceeded"),
            "the first stop reason must be preserved"
        );
        assert!(
            report
                .journal_errors
                .iter()
                .any(|e| e.contains("clean stopped")),
            "the later journal failure must still be recorded: {:?}",
            report.journal_errors
        );
    }

    #[test]
    fn breaker_journal_failure_preserves_breaker_reason_and_records_error() {
        // A breaker stop whose own Skipped-outcome write fails must keep the
        // breaker reason (apply_process_outcome never ran) and still record the
        // journal failure so a partial report reaches the caller.
        let dir = tempdir().unwrap();
        let mut journal = JournalWriter::open_in(dir.path()).unwrap();
        let report = new_clean_report(1);

        let report = stop_after_journal_failure(
            &mut journal,
            report,
            CleanError::BreakerJournalFailure {
                reason: "aborted: 120% circuit breaker exceeded".to_string(),
                detail: "disk full".to_string(),
            },
            &[],
        );

        assert!(report.aborted);
        assert_eq!(
            report.abort_reason.as_deref(),
            Some("aborted: 120% circuit breaker exceeded"),
            "the carried breaker reason must be kept as the abort reason"
        );
        assert!(
            report
                .journal_errors
                .iter()
                .any(|e| e.contains("clean stopped") && e.contains("disk full")),
            "the journal failure must still be recorded: {:?}",
            report.journal_errors
        );
    }

    #[test]
    fn journal_failure_as_first_stop_sets_abort_reason() {
        let dir = tempdir().unwrap();
        let mut journal = JournalWriter::open_in(dir.path()).unwrap();
        let report = new_clean_report(1);

        let report = stop_after_journal_failure(
            &mut journal,
            report,
            CleanError::Invariant("first stop".to_string()),
            &[],
        );

        assert!(report.aborted);
        assert!(
            report
                .abort_reason
                .as_deref()
                .unwrap_or("")
                .contains("first stop"),
            "the journal failure must become the reason when it caused the stop"
        );
    }

    #[test]
    fn root_outcome_carries_child_bytes_for_skipped_and_aborted() {
        let skipped = root_outcome_with_child_bytes(ProcessOutcome::skipped("skip"), 9);
        assert!(
            matches!(skipped, ProcessOutcome::Skipped(reason, 9) if reason == "skip"),
            "a skipped root must carry the completed child bytes"
        );
        let aborted = root_outcome_with_child_bytes(ProcessOutcome::aborted("stop"), 9);
        assert!(
            matches!(aborted, ProcessOutcome::Aborted(reason, 9) if reason == "stop"),
            "an aborted root must carry the completed child bytes"
        );
        let deleted = root_outcome_with_child_bytes(ProcessOutcome::Deleted(13), 9);
        assert!(
            matches!(deleted, ProcessOutcome::Deleted(13)),
            "a deleted root already accounts for the child bytes"
        );
    }

    #[test]
    fn contents_result_carries_deleted_any() {
        let mut state = ContentsState::new("C:\\A\\B\\C");
        assert!(!state.deleted().deleted_any);
        state.deleted_any = true;
        assert!(state.deleted().deleted_any);
        assert!(state.aborted("stop".into()).deleted_any);
        assert!(state.skipped("skip".into()).deleted_any);
    }

    #[test]
    fn stop_outcomes_carry_completed_bytes() {
        let mut state = ContentsState::new("C:\\A\\B\\C");
        state.deleted_bytes = 9;
        let aborted = state.aborted("stop".to_string());
        assert!(matches!(aborted.outcome, ProcessOutcome::Aborted(_, 9)));
        let skipped = state.skipped("skip".to_string());
        assert!(matches!(skipped.outcome, ProcessOutcome::Skipped(_, 9)));
    }

    fn partial_item() -> ValidatedItem {
        validated_item_for_tests(
            "i",
            "t",
            "C:\\A\\B\\C",
            "C:\\A\\B\\C\\child",
            0,
            None,
            ValidatedSource::CustomPath,
        )
    }

    #[test]
    fn fail_after_partial_reports_zero_byte_deletion_when_work_occurred() {
        // A zero-byte file is still a deletion; the decision must not depend
        // on a positive byte count.
        let item = partial_item();
        let mut state = ContentsState::new("C:\\A\\B\\C");
        state.deleted_any = true;
        let mut report = new_clean_report(1);
        let result: Result<(), CleanError> = fail_after_partial(
            &item,
            &state,
            &mut report,
            CleanError::Invariant("boom".into()),
        );
        assert!(result.is_err());
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].status, "deleted");
        assert_eq!(report.items[0].bytes_deleted, 0);
    }

    #[test]
    fn fail_after_partial_skips_result_when_no_entry_was_deleted() {
        // A first-intent failure before any deletion must not fabricate a
        // Deleted result.
        let item = partial_item();
        let state = ContentsState::new("C:\\A\\B\\C");
        let mut report = new_clean_report(1);
        let result: Result<(), CleanError> = fail_after_partial(
            &item,
            &state,
            &mut report,
            CleanError::Invariant("boom".into()),
        );
        assert!(result.is_err());
        assert!(report.items.is_empty());
    }
}
