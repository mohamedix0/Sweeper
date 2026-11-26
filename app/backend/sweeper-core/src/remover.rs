//! Handle-based `Clean` executor (`remover`) with moment-of-deletion re-validation.
//!
//! Sole module allowed to invoke `sweeper-winfs` deletion primitives (enforced by clippy/gate-a).
//!
//! This file is the batch orchestrator and the public API surface. Construction,
//! outcome/journal handling, traversal and ancestor inspection live in focused
//! submodules so no single module accumulates unrelated responsibilities.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::Risk;
use sweeper_winfs::{self, FileIdentity, WinfsError};

use crate::journal::{self, JournalError, JournalWriter};
use crate::paths::CanonicalPath;

mod ancestors;
mod case_variant;
mod clean_progress;
mod constructors;
mod outcomes;
mod prechecks;
mod remover_whole_cache;
mod revalidation;
mod traversal;

#[cfg(test)]
mod tests;

// Internal names from the submodules, re-exported into this module so the
// existing `super::X` import paths in the submodules keep working unchanged.
#[cfg(test)]
use ancestors::{
    ancestor_chain_build_count, ancestor_chain_cache_len, ancestor_chain_retained_cost,
    ANCESTOR_CHAIN_BUILDS,
};
use ancestors::{
    ancestor_chain_cost, inspect_ancestor_prefix, inspect_deep_ancestors, with_ancestor_chain,
    ANCESTOR_CHAIN_RETAINED_COST_BUDGET,
};
use clean_progress::{
    emit_item_progress, stop_and_flush, CleanProgressThrottle, FailureProgress, ItemProgress,
};
use outcomes::{
    apply_process_outcome, journal_deleted_outcome, new_clean_report, record_post_abort_skip,
    root_outcome_with_child_bytes, source_removes_root_after_contents, stop_after_journal_failure,
    ProcessOutcome,
};
use prechecks::{handle_root_contents, precheck_process_item, write_skip};
#[cfg(test)]
use revalidation::{revalidate_handle, RevalidateRequest};
use traversal::{
    clean_directory_contents, delete_path_entry, delete_path_entry_allowing_root,
    CleanContentsResult, CleanTraversalCtx,
};
#[cfg(test)]
use traversal::{fail_after_partial, ContentsState};

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
    /// Volume serial + file index observed when this item was validated.
    pub(crate) expected_identity: Option<FileIdentity>,
    /// Whether moment-of-deletion identity binding is mandatory for this item.
    pub(crate) identity_required: bool,
}

/// Capture the stable filesystem identity (volume serial + file index) for a
/// validated path, for binding into a [`ValidatedItem`] via
/// [`ValidatedItem::with_identity_binding`].
///
/// `None` means no stable identity could be observed (missing/inaccessible
/// path); a bound item with `None` is skipped at moment-of-deletion revalidation.
pub fn capture_file_identity(path: &Path) -> Option<FileIdentity> {
    crate::scanner::file_identity_for_path(path)
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
    pub journal_base_dir: Option<std::path::PathBuf>,
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

/// Process one validated item: prechecks, then either a single entry deletion
/// or the contents-only traversal of a root-style selection.
fn process_item(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    report: &mut CleanReport,
) -> Result<ProcessOutcome, CleanError> {
    let mut ctx = CleanTraversalCtx::new(opts, journal, deleted_total, report);
    if let Some(outcome) = precheck_process_item(item, &mut ctx)? {
        return Ok(outcome);
    }

    let item_path = Path::new(&item.path);
    if !paths_equal_str(&item.path, &item.target_root) {
        return delete_path_entry(item, &item.path, &mut ctx);
    }
    if !item_path.is_dir() {
        // Do not delete root file unexpectedly.
        return write_skip(item, &item.path, "Skipped: target root item", &mut ctx);
    }
    handle_root_contents(item, item_path, &mut ctx)
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
