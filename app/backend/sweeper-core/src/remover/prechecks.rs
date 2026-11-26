//! Preflight checks and outcome-writing helpers for `remover`.
//!
//! These helpers keep `process_item` and `delete_path_entry_inner` below their
//! complexity limits without changing the order of checks, the journal records,
//! the reason strings or the `ProcessOutcome` variants produced by the original
//! inline code.

use std::path::Path;

use sweeper_winfs::{self, HandleInspection, ReparseTagClass, SafeHandle, WinfsError};

use crate::journal;
use crate::paths::CanonicalPath;
use crate::policy;

use super::remover_whole_cache::whole_cache_partial_outcome;
use super::revalidation::{ensure_target_root_safe, revalidate_root_identity, verify_root_anchor};
use super::traversal::DeleteRootAllowingRootRequest;
use super::{
    apply_process_outcome, clean_directory_contents, clear_readonly, delete_dir_handle,
    delete_file_handle, delete_path_entry_allowing_root, inspection_is_directory,
    journal_deleted_outcome, open_for_delete, root_outcome_with_child_bytes,
    source_removes_root_after_contents, would_exceed_circuit_breaker, CleanContentsResult,
    CleanError, CleanTraversalCtx, ProcessOutcome, ValidatedItem, ValidatedSource,
};

/// Shared protected-path predicate for the allowlisted-root construction paths.
pub(super) fn is_protected_root(canon: &CanonicalPath) -> bool {
    if policy::is_subtree_protected(canon) {
        return true;
    }
    if policy::is_ancestor_of_subtree_protected(canon) {
        return true;
    }
    policy::is_protected_container_root(canon)
}

/// Journal an intent plus a Skipped outcome for an item that cannot be started.
pub(super) fn write_skip(
    item: &ValidatedItem,
    path_str: &str,
    reason: &str,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<ProcessOutcome, CleanError> {
    ctx.journal.write_intent(&journal::intent_record(
        &item.id,
        path_str,
        item.size_bytes,
        None,
    ))?;
    ctx.journal.write_outcome(&journal::outcome_record(
        &item.id, path_str, 0, "Skipped", reason, None,
    ))?;
    Ok(ProcessOutcome::skipped(reason))
}

fn is_fatal_target_root_reason(reason: &str) -> bool {
    if reason.contains("escape") {
        return true;
    }
    if reason.contains("swapped to a junction") {
        return true;
    }
    reason.contains("link point") && reason.contains("target root")
}

fn is_fatal_delete_reason(reason: &str) -> bool {
    if reason.contains("escape") {
        return true;
    }
    reason.contains("swapped")
}

fn is_fatal_handle_reason(reason: &str) -> bool {
    if reason.contains("escape") {
        return true;
    }
    reason.contains("target root") && reason.contains("link point")
}

/// Missing target root / unsafe target root / missing item path checks that run
/// before any traversal. `Some` means the caller must return that outcome.
pub(super) fn precheck_process_item(
    item: &ValidatedItem,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<Option<ProcessOutcome>, CleanError> {
    // Missing target root/custom path: zero deletions, parent untouched.
    if !Path::new(&item.target_root).exists() {
        return Ok(Some(write_skip(
            item,
            &item.path,
            "Skipped: missing target root (zero deletions; parent untouched)",
            ctx,
        )?));
    }
    // Revalidate target root/ancestors are not link points at moment of deletion.
    if let Err(reason) = ensure_target_root_safe(&item.target_root) {
        ctx.journal.write_intent(&journal::intent_record(
            &item.id,
            &item.path,
            item.size_bytes,
            None,
        ))?;
        ctx.journal.write_outcome(&journal::outcome_record(
            &item.id, &item.path, 0, "Skipped", &reason, None,
        ))?;
        return Ok(Some(if is_fatal_target_root_reason(&reason) {
            // Target root compromised -> abort entire clean for safety.
            ProcessOutcome::aborted(reason)
        } else {
            ProcessOutcome::skipped(reason)
        }));
    }
    if !Path::new(&item.path).exists() {
        return Ok(Some(write_skip(
            item,
            &item.path,
            "Skipped: missing path",
            ctx,
        )?));
    }
    Ok(None)
}

/// Handle the case where the item is the target root directory itself.
pub(super) fn handle_root_contents(
    item: &ValidatedItem,
    item_path: &Path,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<ProcessOutcome, CleanError> {
    // Fail closed if the root was swapped since preview, and hold the verified
    // root handle for the whole batch. Without the held handle the root could be
    // swapped between this check and the child `open_for_delete` calls (TOCTOU).
    let root_anchor = match verify_root_anchor(item) {
        Ok(anchor) => anchor,
        Err(reason) => return write_skip(item, &item.path, &reason, ctx),
    };
    ctx.set_root_anchor(root_anchor);
    let contents = clean_directory_contents(item, ctx)?;
    // Release the verified root handle before removing the root itself on a
    // separate delete handle.
    drop(ctx.take_root_anchor());
    if let ProcessOutcome::Deleted(bytes) = &contents.outcome {
        let mut fin = RootFinalization {
            item,
            item_path,
            bytes: *bytes,
            contents: &contents,
            ctx,
        };
        return remove_root_after_contents(&mut fin);
    }
    Ok(contents.outcome)
}

/// Inputs for finalizing a root-style item after its contents were cleaned.
///
/// Bundling them keeps the finalization entry point at one argument and lets
/// the decision branches live in small, auditable helpers.
struct RootFinalization<'a, 'b> {
    item: &'a ValidatedItem,
    item_path: &'a Path,
    bytes: u64,
    contents: &'a CleanContentsResult,
    ctx: &'a mut CleanTraversalCtx<'b>,
}

/// Final root safety check after the children were deleted. An abort here keeps
/// the completed child bytes instead of reporting zero.
fn root_safety_outcome(item: &ValidatedItem, bytes: u64) -> Option<ProcessOutcome> {
    ensure_target_root_safe(&item.target_root)
        .err()
        .map(|reason| ProcessOutcome::Aborted(reason, bytes))
}

/// Whether the cleaned root now holds no entries.
fn root_is_empty(item_path: &Path) -> bool {
    sweeper_winfs::read_dir_entries(item_path)
        .map(|v| v.is_empty())
        .unwrap_or(false)
}

/// Outcome when leftover entries remain in the root after the content sweep.
///
/// A whole-cache is all-or-nothing (leftovers mean it was not fully cleaned, so
/// report Skipped with the already-deleted bytes). Tier 3 project directories
/// keep their existing Deleted outcome.
fn leftover_entries_outcome(item: &ValidatedItem, bytes: u64) -> ProcessOutcome {
    if matches!(item.source, ValidatedSource::WholeCache { .. }) {
        whole_cache_partial_outcome(bytes)
    } else {
        ProcessOutcome::Deleted(bytes)
    }
}

/// Finalize a failed root-removal attempt, preserving already-completed child
/// deletions when an entry actually was deleted.
fn finish_root_delete_failure(
    fin: &mut RootFinalization<'_, '_>,
    err: CleanError,
) -> Result<ProcessOutcome, CleanError> {
    // Child deletions already completed even though the final root-removal
    // intent could not be journaled. An already-empty root did no work, so only
    // report a partial deletion when an entry was actually deleted.
    if fin.contents.deleted_any {
        apply_process_outcome(
            fin.item,
            ProcessOutcome::Deleted(fin.bytes),
            &mut *fin.ctx.report,
        );
    }
    Err(err)
}

fn remove_root_after_contents(
    fin: &mut RootFinalization<'_, '_>,
) -> Result<ProcessOutcome, CleanError> {
    // Tier 3 stale project directories and issue #41 whole-cache targets remove
    // the now-cleaned directory root itself.
    if !source_removes_root_after_contents(&fin.item.source) {
        return Ok(ProcessOutcome::Deleted(fin.bytes));
    }
    if let Some(outcome) = root_safety_outcome(fin.item, fin.bytes) {
        return Ok(outcome);
    }
    if !root_is_empty(fin.item_path) {
        return Ok(leftover_entries_outcome(fin.item, fin.bytes));
    }
    let result = delete_path_entry_allowing_root(DeleteRootAllowingRootRequest {
        item: fin.item,
        path_str: &fin.item.path,
        ctx: &mut *fin.ctx,
        base_bytes_deleted: Some(fin.bytes),
        ignore_emptied_dir_age: fin.contents.root_mutated,
    });
    match result {
        // Root was not deleted (Skipped/Aborted): carry the completed child
        // bytes instead of dropping them.
        Ok(root_outcome) => Ok(root_outcome_with_child_bytes(root_outcome, fin.bytes)),
        Err(err) => finish_root_delete_failure(fin, err),
    }
}

/// Target-root and identity rechecks performed after the intent is journaled
/// and before the delete handle is opened.
pub(super) fn precheck_delete(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<Option<ProcessOutcome>, CleanError> {
    if let Err(reason) = ensure_target_root_safe(&item.target_root) {
        ctx.journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        return Ok(Some(if is_fatal_delete_reason(&reason) {
            ProcessOutcome::aborted(reason)
        } else {
            ProcessOutcome::skipped(reason)
        }));
    }

    // Per-deletion identity recheck: a root swapped for a different real
    // directory between traversal batches still passes the link-point check,
    // so re-bind identity immediately before each unlink and abort the batch.
    if let Err(reason) = revalidate_root_identity(item) {
        ctx.journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        return Ok(Some(ProcessOutcome::aborted(reason)));
    }
    Ok(None)
}

pub(super) enum OpenResult {
    Ready(SafeHandle, HandleInspection),
    Done(ProcessOutcome),
}

fn classify_open_error(e: WinfsError) -> String {
    if let WinfsError::Win32 { code, .. } = e {
        if sweeper_winfs::is_not_found(code) {
            "Skipped: missing path".to_string()
        } else if sweeper_winfs::is_lock_like_error(code) {
            "Skipped: locked".to_string()
        } else {
            format!("Skipped: cannot open for delete (os error {code})")
        }
    } else {
        format!("Skipped: cannot open for delete: {e}")
    }
}

/// Open the entry and inspect it on the handle; on failure the outcome is
/// journaled here and the caller returns `Done`.
pub(super) fn open_delete_handle(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<OpenResult, CleanError> {
    let handle = match open_for_delete(Path::new(path_str)) {
        Ok(h) => h,
        Err(e) => {
            let reason = classify_open_error(e);
            ctx.journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(OpenResult::Done(ProcessOutcome::skipped(reason)));
        }
    };

    let inspection = match sweeper_winfs::inspect_handle(&handle) {
        Ok(i) => i,
        Err(e) => {
            let reason = format!("Skipped: handle inspection failed: {e}");
            ctx.journal.write_outcome(&journal::outcome_record(
                &item.id, path_str, 0, "Skipped", &reason, None,
            ))?;
            return Ok(OpenResult::Done(ProcessOutcome::skipped(reason)));
        }
    };
    Ok(OpenResult::Ready(handle, inspection))
}

/// Journal the outcome of a failed moment-of-deletion revalidation and classify
/// it as an abort or a skip exactly as the inline code did.
pub(super) fn handle_revalidation_failure(
    item: &ValidatedItem,
    path_str: &str,
    reason: String,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<ProcessOutcome, CleanError> {
    ctx.journal.write_outcome(&journal::outcome_record(
        &item.id, path_str, 0, "Skipped", &reason, None,
    ))?;
    Ok(if is_fatal_handle_reason(&reason) {
        ProcessOutcome::aborted(reason)
    } else {
        ProcessOutcome::skipped(reason)
    })
}

/// 120% circuit breaker check for the next deletion. `Some` means stop.
pub(super) fn check_circuit_breaker(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
    bytes_accounted: u64,
) -> Result<Option<ProcessOutcome>, CleanError> {
    if !would_exceed_circuit_breaker(
        ctx.opts.preview_estimated_bytes,
        *ctx.deleted_total,
        bytes_accounted,
    ) {
        return Ok(None);
    }
    let reason = "aborted: 120% circuit breaker exceeded".to_string();
    if let Err(err) = ctx.journal.write_outcome(&journal::outcome_record(
        &item.id, path_str, 0, "Skipped", &reason, None,
    )) {
        // Do not lose the breaker reason when its Skipped-outcome write fails:
        // carry it on the error so finalization records both.
        return Err(CleanError::BreakerJournalFailure {
            reason,
            detail: err.to_string(),
        });
    }
    Ok(Some(ProcessOutcome::aborted(reason)))
}

/// Cloud/other reparse tags must be skipped (Link Point allowed only as entry deletion).
pub(super) fn is_skippable_reparse(inspection: &HandleInspection) -> bool {
    matches!(
        inspection.reparse,
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_)
    )
}

pub(super) fn perform_delete(
    handle: &SafeHandle,
    inspection: &HandleInspection,
) -> Result<(), WinfsError> {
    if inspection_is_directory(inspection) {
        delete_dir_handle(handle)
    } else {
        // clear readonly via handle if needed (best-effort)
        let _ = clear_readonly(handle);
        delete_file_handle(handle)
    }
}

fn classify_delete_error(e: WinfsError) -> String {
    match e {
        WinfsError::Win32 { code, .. } if sweeper_winfs::is_not_found(code) => {
            "Skipped: missing path".to_string()
        }
        WinfsError::Win32 { code, .. } if sweeper_winfs::is_lock_like_error(code) => {
            "Skipped: locked".to_string()
        }
        other => format!("Skipped: delete failed: {other}"),
    }
}

/// Journal a best-effort Skipped outcome for a failed delete and return its reason.
pub(super) fn finish_delete_failure(
    item: &ValidatedItem,
    path_str: &str,
    e: WinfsError,
    ctx: &mut CleanTraversalCtx<'_>,
) -> ProcessOutcome {
    let reason = classify_delete_error(e);
    ctx.journal
        .write_outcome_best_effort(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ));
    ProcessOutcome::skipped(reason)
}

/// Journal a successful deletion and fold its bytes into the report result.
pub(super) fn finish_delete_success(
    item: &ValidatedItem,
    path_str: &str,
    bytes_accounted: u64,
    ctx: &mut CleanTraversalCtx<'_>,
) -> ProcessOutcome {
    *ctx.deleted_total = ctx.deleted_total.saturating_add(bytes_accounted);
    let reported = ctx
        .flags
        .base_bytes_deleted
        .unwrap_or(0)
        .saturating_add(bytes_accounted);
    // #135: journal own bytes only; `reported` keeps the full CleanReport total.
    journal_deleted_outcome(ctx.journal, &item.id, path_str, bytes_accounted);
    ProcessOutcome::Deleted(reported)
}
