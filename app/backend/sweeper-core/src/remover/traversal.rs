//! Directory traversal and the handle-based deletion chain for `remover`.
//!
//! The traversal state and per-entry flags are bundled into context structs so
//! each helper stays small. No safety check order, journal record, reason
//! string or partial-deletion accounting rule is changed here.

use std::path::{Path, PathBuf};

use sweeper_winfs::{self, ReparseTagClass, SafeHandle, WinfsError};

use crate::journal::{self, JournalWriter};

use super::outcomes::{apply_process_outcome, ProcessOutcome};
use super::prechecks::{
    check_circuit_breaker, finish_delete_failure, finish_delete_success,
    handle_revalidation_failure, is_skippable_reparse, open_delete_handle, perform_delete,
    precheck_delete, OpenResult,
};
use super::revalidation::{
    ensure_target_root_safe, revalidate_batch_root, revalidate_handle, revalidate_root_identity,
    RevalidateRequest,
};
use super::{
    circuit_breaker_exceeded, inspection_is_directory, paths_equal_str, CleanError, CleanOptions,
    CleanReport, ValidatedItem,
};

/// Result of cleaning one directory's contents.
pub(super) struct CleanContentsResult {
    pub(super) outcome: ProcessOutcome,
    pub(super) root_mutated: bool,
    /// Whether at least one entry was actually deleted (even a zero-byte file).
    /// Distinguishes "empty root, no work" from a completed partial deletion.
    pub(super) deleted_any: bool,
}

fn norm_path_key(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

/// Mutable state accumulated while cleaning a directory's contents.
///
/// Keeping this in one place lets the traversal helpers stay small: each one
/// only records bytes/dirs and decides whether the caller must stop early.
pub(super) struct ContentsState {
    /// Bytes deleted by this contents-clean so far (partial results).
    pub(super) deleted_bytes: u64,
    /// Whether at least one entry was actually deleted (even a zero-byte file).
    pub(super) deleted_any: bool,
    /// Directories whose contents changed, used to bypass the emptied-dir age check.
    mutated_dirs: std::collections::HashSet<String>,
    /// Child directories to remove bottom-up after the file sweep.
    postorder: Vec<PathBuf>,
    /// Normalized key of the contents root.
    root_key: String,
}

impl ContentsState {
    pub(super) fn new(root: &str) -> Self {
        Self {
            deleted_bytes: 0,
            deleted_any: false,
            mutated_dirs: std::collections::HashSet::new(),
            postorder: Vec::new(),
            root_key: norm_path_key(Path::new(root)),
        }
    }

    fn root_mutated(&self) -> bool {
        self.mutated_dirs.contains(&self.root_key)
    }

    pub(super) fn aborted(&self, reason: String) -> CleanContentsResult {
        CleanContentsResult {
            outcome: ProcessOutcome::Aborted(reason, self.deleted_bytes),
            root_mutated: self.root_mutated(),
            deleted_any: self.deleted_any,
        }
    }

    pub(super) fn skipped(&self, reason: String) -> CleanContentsResult {
        CleanContentsResult {
            outcome: ProcessOutcome::Skipped(reason, self.deleted_bytes),
            root_mutated: self.root_mutated(),
            deleted_any: self.deleted_any,
        }
    }

    pub(super) fn deleted(&self) -> CleanContentsResult {
        CleanContentsResult {
            outcome: ProcessOutcome::Deleted(self.deleted_bytes),
            root_mutated: self.root_mutated(),
            deleted_any: self.deleted_any,
        }
    }
}

/// Per-entry flags applied by the next `delete_path_entry_inner` call.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct DeleteEntryFlags {
    /// Whether the item's own root directory may be removed after its contents.
    allow_delete_root: bool,
    /// Bytes already deleted by earlier children, carried into the root outcome.
    pub(super) base_bytes_deleted: Option<u64>,
    /// Whether the entry is a directory emptied during this same traversal.
    ignore_emptied_dir_age: bool,
}

/// Shared mutable state and per-entry flags threaded through the deletion chain.
///
/// Bundling these keeps the traversal helpers small and removes the long
/// positional argument lists (and their lint suppressions) from the delete path.
pub(super) struct CleanTraversalCtx<'a> {
    pub(super) opts: &'a CleanOptions,
    pub(super) journal: &'a mut JournalWriter,
    pub(super) deleted_total: &'a mut u64,
    pub(super) report: &'a mut CleanReport,
    pub(super) flags: DeleteEntryFlags,
    /// Verified root handle held for a root-style deletion batch. Its identity
    /// is re-checked after each child is opened (before deletion) so a root
    /// swap cannot redirect children that were never previewed.
    root_anchor: Option<SafeHandle>,
}

impl<'a> CleanTraversalCtx<'a> {
    pub(super) fn new(
        opts: &'a CleanOptions,
        journal: &'a mut JournalWriter,
        deleted_total: &'a mut u64,
        report: &'a mut CleanReport,
    ) -> Self {
        Self {
            opts,
            journal,
            deleted_total,
            report,
            flags: DeleteEntryFlags::default(),
            root_anchor: None,
        }
    }

    /// Hold a verified root handle for this batch (root-style items only).
    pub(super) fn set_root_anchor(&mut self, anchor: Option<SafeHandle>) {
        self.root_anchor = anchor;
    }

    /// Release the held root handle, e.g. before removing the root itself.
    pub(super) fn take_root_anchor(&mut self) -> Option<SafeHandle> {
        self.root_anchor.take()
    }

    fn root_anchor(&self) -> Option<&SafeHandle> {
        self.root_anchor.as_ref()
    }

    /// Delete one entry with the supplied per-entry flags.
    pub(super) fn delete_entry(
        &mut self,
        item: &ValidatedItem,
        path_str: &str,
        flags: DeleteEntryFlags,
    ) -> Result<ProcessOutcome, CleanError> {
        self.flags = flags;
        delete_path_entry_inner(item, path_str, self)
    }
}

/// A failed target-root recheck during traversal stops the whole clean.
fn check_root(item: &ValidatedItem, state: &ContentsState) -> Option<CleanContentsResult> {
    if let Err(reason) = ensure_target_root_safe(&item.target_root) {
        return Some(state.aborted(reason));
    }
    // A root swapped for a different real directory (no reparse point) passes
    // the link-point check. Re-bind identity before each traversal batch so a
    // mid-walk swap cannot delete children that were never previewed.
    revalidate_root_identity(item)
        .err()
        .map(|reason| state.aborted(reason))
}

/// Apply the deletions already completed, then propagate a fatal error.
///
/// This is what keeps the two-children partial-result guarantee: a later
/// intent/outcome write failure must not erase earlier successful deletions.
pub(super) fn fail_after_partial<T>(
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
    ctx: &mut CleanTraversalCtx<'_>,
    path: &str,
    size: u64,
    reason: &str,
    state: &ContentsState,
) -> Result<(), CleanError> {
    if let Err(err) = ctx
        .journal
        .write_intent(&journal::intent_record(&item.id, path, size, None))
    {
        return fail_after_partial(item, state, &mut *ctx.report, err);
    }
    if let Err(err) = ctx.journal.write_outcome(&journal::outcome_record(
        &item.id, path, 0, "Skipped", reason, None,
    )) {
        return fail_after_partial(item, state, &mut *ctx.report, err);
    }
    Ok(())
}

/// Delete one entry and fold its outcome into the traversal state.
///
/// Returns `Some(result)` only when the caller must stop (abort or the 120%
/// circuit breaker). `mutated_dir` marks the directory whose contents changed.
fn delete_and_apply(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
    state: &mut ContentsState,
    mutated_dir: Option<&Path>,
    ignore_emptied_dir_age: bool,
) -> Result<Option<CleanContentsResult>, CleanError> {
    let flags = DeleteEntryFlags {
        ignore_emptied_dir_age,
        ..DeleteEntryFlags::default()
    };
    let outcome = match ctx.delete_entry(item, path_str, flags) {
        Ok(outcome) => outcome,
        Err(err) => return fail_after_partial(item, state, &mut *ctx.report, err),
    };
    match outcome {
        ProcessOutcome::Deleted(bytes) => {
            state.deleted_bytes = state.deleted_bytes.saturating_add(bytes);
            state.deleted_any = true;
            if let Some(dir) = mutated_dir {
                state.mutated_dirs.insert(norm_path_key(dir));
            }
            if circuit_breaker_exceeded(ctx.opts.preview_estimated_bytes, *ctx.deleted_total) {
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
fn apply_entry(
    item: &ValidatedItem,
    ctx: &mut CleanTraversalCtx<'_>,
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
            ctx,
            state,
            Some(dir),
            false,
        ),
        // Cloud/unknown reparse points are always Skipped and never traversed.
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            journal_skip(
                item,
                ctx,
                &ent.path.to_string_lossy(),
                ent.size,
                "Skipped: reparse point (cloud/unknown)",
                state,
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
                    ctx,
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
    ctx: &mut CleanTraversalCtx<'_>,
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
                journal_skip(item, ctx, &dir.to_string_lossy(), 0, &reason, state)?;
                return Ok(Some(state.skipped(reason)));
            }
        };
        // Only child directories (not the root itself) are deletion candidates.
        if !paths_equal_str(&dir.to_string_lossy(), &item.path) {
            state.postorder.push(dir.clone());
        }
        for ent in entries {
            if let Some(result) = apply_entry(item, ctx, state, stack, &dir, ent)? {
                return Ok(Some(result));
            }
        }
    }
    Ok(None)
}

/// Remove now-empty child directories bottom-up, revalidating safety each time.
fn delete_empty_dirs(
    item: &ValidatedItem,
    ctx: &mut CleanTraversalCtx<'_>,
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
            ctx,
            state,
            parent.as_deref(),
            ignore_emptied_dir_age,
        )? {
            return Ok(Some(result));
        }
    }
    Ok(None)
}

pub(super) fn clean_directory_contents(
    item: &ValidatedItem,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<CleanContentsResult, CleanError> {
    let mut state = ContentsState::new(&item.path);
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(&item.path)];

    if let Some(result) = sweep_entries(item, ctx, &mut state, &mut stack)? {
        return Ok(result);
    }
    if let Some(result) = delete_empty_dirs(item, ctx, &mut state)? {
        return Ok(result);
    }
    // Represent contents-clean as a deleted summary for the root item context.
    Ok(state.deleted())
}

pub(super) fn delete_path_entry(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<ProcessOutcome, CleanError> {
    ctx.delete_entry(item, path_str, DeleteEntryFlags::default())
}

pub(super) fn delete_path_entry_allowing_root(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
    base_bytes_deleted: Option<u64>,
    ignore_emptied_dir_age: bool,
) -> Result<ProcessOutcome, CleanError> {
    ctx.delete_entry(
        item,
        path_str,
        DeleteEntryFlags {
            allow_delete_root: true,
            base_bytes_deleted,
            ignore_emptied_dir_age,
        },
    )
}

fn delete_path_entry_inner(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<ProcessOutcome, CleanError> {
    // Intent before each deletion batch/item.
    ctx.journal.write_intent(&journal::intent_record(
        &item.id,
        path_str,
        item.size_bytes,
        None,
    ))?;

    if let Some(outcome) = precheck_delete(item, path_str, ctx)? {
        return Ok(outcome);
    }

    let (handle, inspection) = match open_delete_handle(item, path_str, ctx)? {
        OpenResult::Ready(handle, inspection) => (handle, inspection),
        OpenResult::Done(outcome) => return Ok(outcome),
    };

    // Moment-of-deletion re-validation on opened handle.
    if let Err(reason) = revalidate_handle(&RevalidateRequest {
        item,
        requested_path: path_str,
        inspection: &inspection,
        opts: ctx.opts,
        allow_delete_root: ctx.flags.allow_delete_root,
        ignore_emptied_dir_age: ctx.flags.ignore_emptied_dir_age,
    }) {
        return handle_revalidation_failure(item, path_str, reason, ctx);
    }

    // Root-swap TOCTOU guard: the child handle is open now, so re-verify the
    // root path against the handle held for this batch. A swap before the child
    // open is caught here; a swap after this check cannot redirect the already
    // open child handle. The held handle keeps the batch bound to the exact
    // root object whose identity was verified at the start.
    let batch_root_check = ctx
        .root_anchor()
        .map(|anchor| revalidate_batch_root(item, anchor));
    if let Some(Err(reason)) = batch_root_check {
        return handle_revalidation_failure(item, path_str, reason, ctx);
    }

    // Track actual deleted bytes for circuit breaker based on handle-observed file size (or item estimate).
    let bytes_accounted = match get_handle_logical_size(&handle, &inspection) {
        Ok(Some(sz)) => sz,
        Ok(None) => item.size_bytes,
        Err(_) => item.size_bytes,
    };
    if let Some(outcome) = check_circuit_breaker(item, path_str, ctx, bytes_accounted)? {
        return Ok(outcome);
    }

    if is_skippable_reparse(&inspection) {
        let reason = "Skipped: reparse point (cloud/unknown)".to_string();
        ctx.journal.write_outcome(&journal::outcome_record(
            &item.id, path_str, 0, "Skipped", &reason, None,
        ))?;
        return Ok(ProcessOutcome::skipped(reason));
    }

    match perform_delete(&handle, &inspection) {
        Ok(()) => Ok(finish_delete_success(item, path_str, bytes_accounted, ctx)),
        Err(e) => Ok(finish_delete_failure(item, path_str, e, ctx)),
    }
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
