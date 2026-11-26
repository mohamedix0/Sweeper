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
    /// First reason an entry was skipped. Kept so a root whose work was
    /// entirely skipped reports Skipped instead of a fabricated `Deleted(0)`.
    /// A genuinely empty root records nothing here.
    skipped_reason: Option<String>,
}

impl ContentsState {
    pub(super) fn new(root: &str) -> Self {
        Self {
            deleted_bytes: 0,
            deleted_any: false,
            mutated_dirs: std::collections::HashSet::new(),
            postorder: Vec::new(),
            root_key: norm_path_key(Path::new(root)),
            skipped_reason: None,
        }
    }

    /// Record the first skip reason. Later skips never overwrite it.
    pub(super) fn record_skipped(&mut self, reason: String) {
        if self.skipped_reason.is_none() {
            self.skipped_reason = Some(reason);
        }
    }

    /// Final outcome for the contents root.
    ///
    /// When no entry was deleted but an entry was skipped, the root must be
    /// reported as Skipped with that reason. `deleted_any` (not a zero byte
    /// count) distinguishes this from a zero-byte file that was really removed:
    /// a genuinely empty root has no skip reason and still finalizes as
    /// `Deleted(0)`.
    pub(super) fn final_outcome(&self) -> CleanContentsResult {
        if !self.deleted_any {
            if let Some(reason) = &self.skipped_reason {
                return self.skipped(reason.clone());
            }
        }
        self.deleted()
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

/// Named inputs for journaling one Skipped entry (unreadable directory or
/// reparse placeholder).
struct JournalSkipRequest<'a, 'b> {
    item: &'a ValidatedItem,
    ctx: &'a mut CleanTraversalCtx<'b>,
    path: &'a str,
    size: u64,
    reason: &'a str,
    state: &'a mut ContentsState,
}

/// Journal an intent plus a Skipped outcome for one entry. A fatal write
/// preserves already-completed deletions.
fn journal_skip(req: JournalSkipRequest<'_, '_>) -> Result<(), CleanError> {
    if let Err(err) = req.ctx.journal.write_intent(&journal::intent_record(
        &req.item.id,
        req.path,
        req.size,
        None,
    )) {
        return fail_after_partial(req.item, &*req.state, &mut *req.ctx.report, err);
    }
    if let Err(err) = req.ctx.journal.write_outcome(&journal::outcome_record(
        &req.item.id,
        req.path,
        0,
        "Skipped",
        req.reason,
        None,
    )) {
        return fail_after_partial(req.item, &*req.state, &mut *req.ctx.report, err);
    }
    // Keep the skip reason for root finalization: a root whose entries were all
    // skipped must not be reported as a zero-byte deletion.
    req.state.record_skipped(req.reason.to_string());
    Ok(())
}

/// Named inputs for deleting one entry and folding its outcome into the
/// traversal state.
struct DeleteAndApplyRequest<'a, 'b> {
    item: &'a ValidatedItem,
    path_str: &'a str,
    ctx: &'a mut CleanTraversalCtx<'b>,
    state: &'a mut ContentsState,
    /// Directory whose contents changed, if this entry is a file/link child.
    mutated_dir: Option<&'a Path>,
    ignore_emptied_dir_age: bool,
}

/// Delete one entry and fold its outcome into the traversal state.
///
/// Returns `Some(result)` only when the caller must stop (abort or the 120%
/// circuit breaker). `mutated_dir` marks the directory whose contents changed.
fn delete_and_apply(
    req: DeleteAndApplyRequest<'_, '_>,
) -> Result<Option<CleanContentsResult>, CleanError> {
    let flags = DeleteEntryFlags {
        ignore_emptied_dir_age: req.ignore_emptied_dir_age,
        ..DeleteEntryFlags::default()
    };
    let outcome = match req.ctx.delete_entry(req.item, req.path_str, flags) {
        Ok(outcome) => outcome,
        Err(err) => return fail_after_partial(req.item, req.state, &mut *req.ctx.report, err),
    };
    match outcome {
        ProcessOutcome::Deleted(bytes) => {
            req.state.deleted_bytes = req.state.deleted_bytes.saturating_add(bytes);
            req.state.deleted_any = true;
            if let Some(dir) = req.mutated_dir {
                req.state.mutated_dirs.insert(norm_path_key(dir));
            }
            if circuit_breaker_exceeded(
                req.ctx.opts.preview_estimated_bytes,
                *req.ctx.deleted_total,
            ) {
                return Ok(Some(
                    req.state
                        .aborted("aborted: 120% circuit breaker exceeded".to_string()),
                ));
            }
        }
        ProcessOutcome::Aborted(reason, _) => return Ok(Some(req.state.aborted(reason))),
        ProcessOutcome::Skipped(reason, _) => req.state.record_skipped(reason),
    }
    Ok(None)
}

/// Named inputs for classifying one enumerated directory entry.
struct ApplyEntryRequest<'a, 'b> {
    item: &'a ValidatedItem,
    ctx: &'a mut CleanTraversalCtx<'b>,
    state: &'a mut ContentsState,
    stack: &'a mut Vec<PathBuf>,
    dir: &'a Path,
    ent: sweeper_winfs::DirEntry,
}

/// Classify and handle one enumerated entry: unlink link points (never
/// traverse them), skip reparse placeholders, queue real child dirs, delete
/// real files.
fn apply_entry(req: ApplyEntryRequest<'_, '_>) -> Result<Option<CleanContentsResult>, CleanError> {
    let ApplyEntryRequest {
        item,
        ctx,
        state,
        stack,
        dir,
        ent,
    } = req;
    match ent.reparse {
        // Unlink only the link entry itself.
        ReparseTagClass::LinkPoint => delete_and_apply(DeleteAndApplyRequest {
            item,
            path_str: &ent.path.to_string_lossy(),
            ctx,
            state,
            mutated_dir: Some(dir),
            ignore_emptied_dir_age: false,
        }),
        // Cloud/unknown reparse points are always Skipped and never traversed.
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            let path = ent.path.to_string_lossy();
            journal_skip(JournalSkipRequest {
                item,
                ctx,
                path: &path,
                size: ent.size,
                reason: "Skipped: reparse point (cloud/unknown)",
                state,
            })?;
            Ok(None)
        }
        ReparseTagClass::NotReparse => {
            if ent.is_dir {
                stack.push(ent.path);
                Ok(None)
            } else {
                delete_and_apply(DeleteAndApplyRequest {
                    item,
                    path_str: &ent.path.to_string_lossy(),
                    ctx,
                    state,
                    mutated_dir: Some(dir),
                    ignore_emptied_dir_age: false,
                })
            }
        }
    }
}

/// Result of reading one directory during the file sweep.
enum SweepDirRead {
    Entries(Vec<sweeper_winfs::DirEntry>),
    /// Directory was unreadable: a Skipped outcome was journaled and the sweep
    /// must stop with this result.
    Stop(CleanContentsResult),
    /// The queued directory is no longer an ordinary directory (it became a
    /// Link Point, a cloud/unknown reparse point, or its resolved path moved).
    /// It was journaled as Skipped and must never be enumerated by name.
    Skip,
}

/// Decide whether a queued directory's inspected handle is still safe to
/// enumerate. Opening with `FILE_FLAG_OPEN_REPARSE_POINT` keeps the handle bound
/// to the link entry itself, so a swap to a junction/symlink is visible as a
/// Link Point on the handle instead of being silently followed into its target.
/// Only the reparse tag is read here: resolving the handle's full path for every
/// directory would be quadratic on deep trees. Interior junction redirects are
/// still rejected when each child is deleted, where `revalidate_handle`
/// compares the child's handle final path with its confirmed path.
pub(super) fn queued_dir_skip_reason(reparse: ReparseTagClass, dir: &Path) -> Option<String> {
    let path = dir.to_string_lossy();
    if reparse.is_link_point() {
        return Some(format!(
            "Skipped: queued directory became a link point during contents clean: {path}"
        ));
    }
    if !matches!(reparse, ReparseTagClass::NotReparse) {
        return Some(format!(
            "Skipped: reparse point (cloud/unknown) on queued directory during contents clean: {path}"
        ));
    }
    None
}

/// Read one directory's entries, journaling a Skipped outcome when unreadable.
///
/// The queued path is opened on a reparse-safe handle and its reparse tag
/// inspected immediately before it is enumerated; the handle is held across the
/// enumeration. A directory queued earlier as an ordinary child can otherwise
/// be swapped for a junction before its turn, and name-based enumeration would
/// then follow the Link Point outside the allowlisted root. Any interior-junction
/// swap after this check is still caught by `revalidate_handle` when each
/// enumerated child is deleted at the moment of deletion.
fn read_sweep_dir(
    item: &ValidatedItem,
    ctx: &mut CleanTraversalCtx<'_>,
    state: &mut ContentsState,
    dir: &Path,
) -> Result<SweepDirRead, CleanError> {
    let path = dir.to_string_lossy().to_string();
    let handle = match sweeper_winfs::open_for_inspection(dir) {
        Ok(handle) => handle,
        Err(e) => {
            let reason =
                format!("Skipped: cannot inspect queued directory during contents clean: {e}");
            journal_skip(JournalSkipRequest {
                item,
                ctx,
                path: &path,
                size: 0,
                reason: &reason,
                state,
            })?;
            return Ok(SweepDirRead::Skip);
        }
    };
    let reparse = match sweeper_winfs::inspect_reparse_point(&handle) {
        Ok(reparse) => reparse,
        Err(e) => {
            let reason =
                format!("Skipped: queued directory inspection failed during contents clean: {e}");
            journal_skip(JournalSkipRequest {
                item,
                ctx,
                path: &path,
                size: 0,
                reason: &reason,
                state,
            })?;
            return Ok(SweepDirRead::Skip);
        }
    };
    if let Some(reason) = queued_dir_skip_reason(reparse, dir) {
        journal_skip(JournalSkipRequest {
            item,
            ctx,
            path: &path,
            size: 0,
            reason: &reason,
            state,
        })?;
        return Ok(SweepDirRead::Skip);
    }

    let entries = match sweeper_winfs::read_dir_entries(dir) {
        Ok(entries) => entries,
        Err(e) => {
            let reason = format!("Skipped: unreadable directory during contents clean: {e}");
            journal_skip(JournalSkipRequest {
                item,
                ctx,
                path: &path,
                size: 0,
                reason: &reason,
                state,
            })?;
            return Ok(SweepDirRead::Stop(state.skipped(reason)));
        }
    };
    // Hold the inspected handle until enumeration completes so the verified
    // directory object stays open for the whole check/enumerate sequence.
    drop(handle);
    Ok(SweepDirRead::Entries(entries))
}

/// Named inputs for applying all entries read from one swept directory.
struct ApplyEnumeratedEntriesRequest<'a, 'b> {
    item: &'a ValidatedItem,
    ctx: &'a mut CleanTraversalCtx<'b>,
    state: &'a mut ContentsState,
    stack: &'a mut Vec<PathBuf>,
    dir: &'a Path,
    entries: Vec<sweeper_winfs::DirEntry>,
}

/// Apply each enumerated entry until one asks the caller to stop.
fn apply_enumerated_entries(
    req: ApplyEnumeratedEntriesRequest<'_, '_>,
) -> Result<Option<CleanContentsResult>, CleanError> {
    let ApplyEnumeratedEntriesRequest {
        item,
        ctx,
        state,
        stack,
        dir,
        entries,
    } = req;
    for ent in entries {
        let step = apply_entry(ApplyEntryRequest {
            item,
            ctx: &mut *ctx,
            state: &mut *state,
            stack: &mut *stack,
            dir,
            ent,
        })?;
        if step.is_some() {
            return Ok(step);
        }
    }
    Ok(None)
}

/// Named inputs for one iteration of the directory sweep.
struct SweepDirRequest<'a, 'b> {
    item: &'a ValidatedItem,
    ctx: &'a mut CleanTraversalCtx<'b>,
    state: &'a mut ContentsState,
    stack: &'a mut Vec<PathBuf>,
    dir: &'a Path,
}

/// Revalidate, read and process a single directory from the sweep stack.
fn sweep_one_dir(req: SweepDirRequest<'_, '_>) -> Result<Option<CleanContentsResult>, CleanError> {
    let SweepDirRequest {
        item,
        ctx,
        state,
        stack,
        dir,
    } = req;
    if let Some(result) = check_root(item, state) {
        return Ok(Some(result));
    }
    let entries = match read_sweep_dir(item, ctx, state, dir)? {
        SweepDirRead::Entries(entries) => entries,
        SweepDirRead::Stop(result) => return Ok(Some(result)),
        // The queued directory became a Link Point (or otherwise unsafe): it
        // was journaled as Skipped and must not be enumerated or queued for
        // bottom-up removal. Siblings continue to be swept.
        SweepDirRead::Skip => return Ok(None),
    };
    // Only child directories (not the root itself) are deletion candidates.
    if !paths_equal_str(&dir.to_string_lossy(), &item.path) {
        state.postorder.push(dir.to_path_buf());
    }
    apply_enumerated_entries(ApplyEnumeratedEntriesRequest {
        item,
        ctx,
        state,
        stack,
        dir,
        entries,
    })
}

/// Iteratively sweep files/link entries without following link points.
fn sweep_entries(
    item: &ValidatedItem,
    ctx: &mut CleanTraversalCtx<'_>,
    state: &mut ContentsState,
    stack: &mut Vec<PathBuf>,
) -> Result<Option<CleanContentsResult>, CleanError> {
    while let Some(dir) = stack.pop() {
        if let Some(result) = sweep_one_dir(SweepDirRequest {
            item,
            ctx: &mut *ctx,
            state: &mut *state,
            stack: &mut *stack,
            dir: &dir,
        })? {
            return Ok(Some(result));
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
        if let Some(result) = delete_and_apply(DeleteAndApplyRequest {
            item,
            path_str: &dir.to_string_lossy(),
            ctx,
            state,
            mutated_dir: parent.as_deref(),
            ignore_emptied_dir_age,
        })? {
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
    // An entirely skipped root is not a zero-byte deletion. A genuinely empty
    // root (no skipped entry) still finalizes as Deleted with zero bytes.
    Ok(state.final_outcome())
}

pub(super) fn delete_path_entry(
    item: &ValidatedItem,
    path_str: &str,
    ctx: &mut CleanTraversalCtx<'_>,
) -> Result<ProcessOutcome, CleanError> {
    ctx.delete_entry(item, path_str, DeleteEntryFlags::default())
}

/// Named inputs for a root-style deletion after its contents were cleaned.
struct DeleteRootRequest<'a, 'b> {
    item: &'a ValidatedItem,
    path_str: &'a str,
    ctx: &'a mut CleanTraversalCtx<'b>,
    flags: DeleteEntryFlags,
}

/// Core root-style deletion; the positional entry point below only assembles
/// the request, so the deletion chain itself has a single named-field argument.
fn delete_root_entry(req: DeleteRootRequest<'_, '_>) -> Result<ProcessOutcome, CleanError> {
    req.ctx.delete_entry(req.item, req.path_str, req.flags)
}

/// Named inputs for a root-style deletion that may remove the root itself.
pub(super) struct DeleteRootAllowingRootRequest<'a, 'b> {
    pub(super) item: &'a ValidatedItem,
    pub(super) path_str: &'a str,
    pub(super) ctx: &'a mut CleanTraversalCtx<'b>,
    pub(super) base_bytes_deleted: Option<u64>,
    pub(super) ignore_emptied_dir_age: bool,
}

pub(super) fn delete_path_entry_allowing_root(
    req: DeleteRootAllowingRootRequest<'_, '_>,
) -> Result<ProcessOutcome, CleanError> {
    let DeleteRootAllowingRootRequest {
        item,
        path_str,
        ctx,
        base_bytes_deleted,
        ignore_emptied_dir_age,
    } = req;
    delete_root_entry(DeleteRootRequest {
        item,
        path_str,
        ctx,
        flags: DeleteEntryFlags {
            allow_delete_root: true,
            base_bytes_deleted,
            ignore_emptied_dir_age,
        },
    })
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
    let bytes_accounted = account_handle_size(&handle, &inspection, item.size_bytes);
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

/// Resolve the handle-observed size, falling back to the item estimate. The
/// fallback is unchanged from the original three-arm match.
fn account_handle_size(
    handle: &sweeper_winfs::SafeHandle,
    inspection: &sweeper_winfs::HandleInspection,
    fallback: u64,
) -> u64 {
    match get_handle_logical_size(handle, inspection) {
        Ok(Some(size)) => size,
        Ok(None) | Err(_) => fallback,
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
