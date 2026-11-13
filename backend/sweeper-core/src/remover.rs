//! Handle-based `Clean` executor (`remover`) with moment-of-deletion re-validation.
//!
//! Sole module allowed to invoke `sweeper-winfs` deletion primitives (enforced by clippy/gate-a).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::Risk;
use sweeper_winfs::{self, ReparseTagClass, WinfsError};

use crate::journal::{self, JournalError, JournalWriter};
use crate::paths::CanonicalPath;
use crate::policy;

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
}

/// Execute handle-based clean for validated items.
///
/// Safety-critical: revalidates on opened handles, journals intent before deletion and outcomes after,
/// aborts on 120% overrun or target-root escape, and uses an abort-on-panic guard.
pub fn execute_clean(
    items: Vec<ValidatedItem>,
    opts: CleanOptions,
) -> Result<CleanReport, CleanError> {
    let _panic_guard = AbortOnPanicGuard;
    let base_dir = opts
        .journal_base_dir
        .clone()
        .unwrap_or_else(journal::default_logs_base_dir);
    let mut journal = JournalWriter::open_in(&base_dir)?;

    let mut report = CleanReport {
        items: Vec::with_capacity(items.len()),
        bytes_freed: 0,
        skipped_count: 0,
        aborted: false,
        abort_reason: None,
    };
    let mut deleted_total: u64 = 0;

    for item in items {
        if report.aborted {
            // Once aborted, remaining items are skipped with reason.
            let reason = report
                .abort_reason
                .clone()
                .unwrap_or_else(|| "aborted".to_string());
            record_skip(&mut journal, &mut report, &item, &reason)?;
            continue;
        }

        let outcome = process_item(&item, &opts, &mut journal, &mut deleted_total, &report)?;
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
            }
            ProcessOutcome::Skipped(reason) => {
                report.items.push(ItemCleanResult {
                    id: item.id.clone(),
                    path: item.path.clone(),
                    status: "Skipped".to_string(),
                    reason,
                    bytes_deleted: 0,
                });
                report.skipped_count += 1;
            }
            ProcessOutcome::Aborted(reason) => {
                report.aborted = true;
                report.abort_reason = Some(reason.clone());
                report.items.push(ItemCleanResult {
                    id: item.id.clone(),
                    path: item.path.clone(),
                    status: "Skipped".to_string(),
                    reason,
                    bytes_deleted: 0,
                });
                report.skipped_count += 1;
            }
        }
    }

    Ok(report)
}

enum ProcessOutcome {
    Deleted(u64),
    Skipped(String),
    Aborted(String),
}

fn process_item(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
    _report: &CleanReport,
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
        return Ok(ProcessOutcome::Skipped(reason));
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
            || reason.contains("link point") && reason.contains("target root")
        {
            // Target root compromised -> abort entire clean for safety.
            return Ok(ProcessOutcome::Aborted(reason));
        }
        return Ok(ProcessOutcome::Skipped(reason));
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
        return Ok(ProcessOutcome::Skipped(reason));
    }

    // Determine if item is the target root itself (custom path root must be kept: clean contents only).
    let is_root_itself = paths_equal_str(&item.path, &item.target_root);
    if is_root_itself {
        // For contents-only semantics, if selected item is root directory, delete its children only.
        if item_path.is_dir() {
            return clean_directory_contents(item, opts, journal, deleted_total);
        }
        // Should not occur for custom-path root semantics, but do not delete root file unexpectedly.
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
        return Ok(ProcessOutcome::Skipped(reason));
    }

    delete_path_entry(item, &item.path, opts, journal, deleted_total)
}

fn clean_directory_contents(
    item: &ValidatedItem,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
) -> Result<ProcessOutcome, CleanError> {
    // Iterative traversal of contents, deleting files and empty dirs/link entries without following link points.
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(&item.path)];
    let mut postorder: Vec<PathBuf> = Vec::new();
    let mut any_deleted_bytes: u64 = 0;

    while let Some(dir) = stack.pop() {
        // Re-check root safety during traversal.
        if let Err(reason) = ensure_target_root_safe(&item.target_root) {
            return Ok(ProcessOutcome::Aborted(reason));
        }
        let entries = match sweeper_winfs::read_dir_entries(&dir) {
            Ok(e) => e,
            Err(e) => {
                journal.write_intent(&journal::intent_record(
                    &item.id,
                    &dir.to_string_lossy(),
                    0,
                    None,
                ))?;
                let reason = format!("Skipped: unreadable directory during contents clean: {e}");
                journal.write_outcome(&journal::outcome_record(
                    &item.id,
                    &dir.to_string_lossy(),
                    0,
                    "Skipped",
                    &reason,
                    None,
                ))?;
                return Ok(ProcessOutcome::Skipped(reason));
            }
        };
        // Record directory for possible deletion later only if it is not the root itself.
        if !paths_equal_str(&dir.to_string_lossy(), &item.path) {
            postorder.push(dir.clone());
        }
        for ent in entries {
            match ent.reparse {
                ReparseTagClass::LinkPoint => {
                    // Unlink only the link entry itself.
                    match delete_path_entry(
                        item,
                        &ent.path.to_string_lossy(),
                        opts,
                        journal,
                        deleted_total,
                    )? {
                        ProcessOutcome::Deleted(b) => {
                            any_deleted_bytes = any_deleted_bytes.saturating_add(b);
                            if circuit_breaker_exceeded(
                                opts.preview_estimated_bytes,
                                *deleted_total,
                            ) {
                                return Ok(ProcessOutcome::Aborted(
                                    "aborted: 120% circuit breaker exceeded".to_string(),
                                ));
                            }
                        }
                        ProcessOutcome::Aborted(r) => return Ok(ProcessOutcome::Aborted(r)),
                        ProcessOutcome::Skipped(_) => {}
                    }
                }
                ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
                    // Skip reparse entries (cloud/other). Do not traverse.
                    journal.write_intent(&journal::intent_record(
                        &item.id,
                        &ent.path.to_string_lossy(),
                        ent.size,
                        None,
                    ))?;
                    let reason = "Skipped: reparse point (cloud/unknown)".to_string();
                    journal.write_outcome(&journal::outcome_record(
                        &item.id,
                        &ent.path.to_string_lossy(),
                        0,
                        "Skipped",
                        &reason,
                        None,
                    ))?;
                }
                ReparseTagClass::NotReparse => {
                    if ent.is_dir {
                        stack.push(ent.path);
                    } else {
                        match delete_path_entry(
                            item,
                            &ent.path.to_string_lossy(),
                            opts,
                            journal,
                            deleted_total,
                        )? {
                            ProcessOutcome::Deleted(b) => {
                                any_deleted_bytes = any_deleted_bytes.saturating_add(b);
                                if circuit_breaker_exceeded(
                                    opts.preview_estimated_bytes,
                                    *deleted_total,
                                ) {
                                    return Ok(ProcessOutcome::Aborted(
                                        "aborted: 120% circuit breaker exceeded".to_string(),
                                    ));
                                }
                            }
                            ProcessOutcome::Aborted(r) => return Ok(ProcessOutcome::Aborted(r)),
                            ProcessOutcome::Skipped(_) => {}
                        }
                    }
                }
            }
        }
    }

    // Delete now-empty child directories bottom-up (skip if not empty or revalidated unsafe).
    for dir in postorder.into_iter().rev() {
        if let Err(reason) = ensure_target_root_safe(&item.target_root) {
            return Ok(ProcessOutcome::Aborted(reason));
        }
        // Skip if directory not empty.
        let is_empty = sweeper_winfs::read_dir_entries(&dir)
            .map(|v| v.is_empty())
            .unwrap_or(false);
        if !is_empty {
            continue;
        }
        // Delete directory entry itself (must remain within target root and pass revalidation).
        match delete_path_entry(item, &dir.to_string_lossy(), opts, journal, deleted_total)? {
            ProcessOutcome::Deleted(b) => {
                any_deleted_bytes = any_deleted_bytes.saturating_add(b);
                if circuit_breaker_exceeded(opts.preview_estimated_bytes, *deleted_total) {
                    return Ok(ProcessOutcome::Aborted(
                        "aborted: 120% circuit breaker exceeded".to_string(),
                    ));
                }
            }
            ProcessOutcome::Aborted(r) => return Ok(ProcessOutcome::Aborted(r)),
            ProcessOutcome::Skipped(_) => {}
        }
    }

    // Represent contents-clean as deleted summary for the root item context.
    Ok(ProcessOutcome::Deleted(any_deleted_bytes))
}

fn delete_path_entry(
    item: &ValidatedItem,
    path_str: &str,
    opts: &CleanOptions,
    journal: &mut JournalWriter,
    deleted_total: &mut u64,
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
            ProcessOutcome::Aborted(reason)
        } else {
            ProcessOutcome::Skipped(reason)
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
            return Ok(ProcessOutcome::Skipped(reason));
        }
    };

    let inspection = match sweeper_winfs::inspect_handle(&handle) {
        Ok(i) => i,
        Err(e) => {
            let reason = format!("Skipped: handle inspection failed: {e}");
            journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(ProcessOutcome::Skipped(reason));
        }
    };

    // Moment-of-deletion re-validation on opened handle.
    if let Err(reason) = revalidate_handle(item, path_str, &inspection, opts) {
        journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        if reason.contains("escape")
            || reason.contains("target root") && reason.contains("link point")
        {
            return Ok(ProcessOutcome::Aborted(reason));
        }
        return Ok(ProcessOutcome::Skipped(reason));
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
        journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        return Ok(ProcessOutcome::Aborted(reason));
    }

    // Cloud/other reparse tags must be skipped (Link Point allowed only as entry deletion).
    match inspection.reparse {
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            let reason = "Skipped: reparse point (cloud/unknown)".to_string();
            journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(ProcessOutcome::Skipped(reason));
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
            journal.write_outcome(&journal::outcome_record(
                &item.id,
                path_str,
                bytes_accounted,
                "deleted",
                "",
                None,
            ))?;
            Ok(ProcessOutcome::Deleted(bytes_accounted))
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
            journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            Ok(ProcessOutcome::Skipped(reason))
        }
    }
}

fn revalidate_handle(
    item: &ValidatedItem,
    _requested_path: &str,
    inspection: &sweeper_winfs::HandleInspection,
    opts: &CleanOptions,
) -> Result<(), String> {
    let final_path = strip_extended_prefix(&inspection.final_path);
    let final_canon = CanonicalPath::parse_lexical(&final_path)
        .map_err(|e| format!("Skipped: final path invalid: {e}"))?;
    let target_root = CanonicalPath::parse_lexical(&item.target_root)
        .map_err(|e| format!("Skipped: target root invalid: {e}"))?;

    // 1) Allowlist / validated Custom Path match (encoded in ValidatedItem source).
    match &item.source {
        ValidatedSource::AllowlistedTarget { .. } | ValidatedSource::CustomPath => {}
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
        // Must not delete the target/custom root itself.
        return Err("Skipped: target root must not be deleted".to_string());
    }
    if path_has_link_point_ancestor(&final_canon, &target_root, true)
        .map_err(|e| format!("Skipped: ancestor reparse check failed: {e}"))?
    {
        return Err("aborted: target root/ancestor link point detected".to_string());
    }

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

fn path_has_link_point_ancestor(
    path: &CanonicalPath,
    boundary_root: &CanonicalPath,
    exclude_final_path: bool,
) -> Result<bool, String> {
    let drive = path.drive_prefix();
    let mut cur = PathBuf::from(format!(r"{}\", drive));
    for seg in path.segments() {
        cur.push(seg);
        let Ok(cur_canon) = CanonicalPath::parse_lexical(&cur.to_string_lossy()) else {
            continue;
        };
        if exclude_final_path && cur_canon.is_same_path(path) {
            continue;
        }
        // If checking a specific boundary root context, we still must detect if boundary_root itself or
        // any of its ancestors (down to this point) is a link point.
        let _ = boundary_root; // boundary considered by callers
        match sweeper_winfs::open_for_inspection(&cur)
            .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        {
            Ok(ReparseTagClass::LinkPoint) => return Ok(true),
            Ok(_) => {}
            Err(WinfsError::Win32 { code, .. }) if sweeper_winfs::is_not_found(code) => {
                return Ok(false)
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
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
