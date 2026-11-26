//! Moment-of-deletion revalidation helpers for `remover`.
//!
//! Every check here runs against the opened handle (or a freshly opened
//! inspection handle) at deletion time. Identity binding, confirmed-path
//! resolution, protected-path containment, link-point ancestors, reparse tags
//! and Minimum Age are the original `remover.rs` checks, regrouped here so the
//! parent module stays within its size limit. Error strings and check order are
//! unchanged.

use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use sweeper_winfs::{self, FileIdentity, HandleInspection, ReparseTagClass, SafeHandle};

use crate::paths::CanonicalPath;
use crate::policy;

use super::case_variant::reject_case_distinct_sibling;
use super::remover_whole_cache::age_check_applies;
#[cfg(test)]
use super::ANCESTOR_CHAIN_BUILDS;
use super::{
    ancestor_chain_cost, filetime_to_unix_secs, inspect_ancestor_prefix, inspect_deep_ancestors,
    paths_equal_str, source_removes_root_after_contents, strip_extended_prefix,
    with_ancestor_chain, CleanOptions, ValidatedItem, ValidatedSource,
    ANCESTOR_CHAIN_RETAINED_COST_BUDGET,
};

/// Single definition of the fail-closed identity rule shared by the handle-level
/// and root-level revalidation paths.
pub(super) const FILE_CHANGED_SINCE_PREVIEW: &str = "Skipped: file changed since preview";

fn identity_matches(item: &ValidatedItem, observed: FileIdentity) -> bool {
    item.expected_identity == Some(observed)
}

/// Authorized source check. All `ValidatedItem` construction paths produce one
/// of these sources; the exhaustive match keeps that invariant explicit.
fn ensure_authorized_source(source: &ValidatedSource) {
    match source {
        ValidatedSource::AllowlistedTarget { .. }
        | ValidatedSource::CustomPath
        | ValidatedSource::Tier3ProjectDirectory { .. }
        | ValidatedSource::Tier3RecycleBin
        | ValidatedSource::WholeCache { .. } => {}
    }
}

/// Protected Path exclusion (Container & Subtree via canonical handle identity).
fn reject_protected_path(final_canon: &CanonicalPath) -> Result<(), String> {
    if policy::is_subtree_protected(final_canon)
        || policy::is_ancestor_of_subtree_protected(final_canon)
    {
        return Err("Skipped: protected path".to_string());
    }
    if policy::is_protected_container_root(final_canon) {
        return Err("Skipped: protected container root".to_string());
    }
    Ok(())
}

/// Final path containment within the Target root, plus the link-point checks
/// for the root itself and its ancestors.
fn check_root_containment(
    final_canon: &CanonicalPath,
    target_root: &CanonicalPath,
    allow_delete_root: bool,
    source: &ValidatedSource,
) -> Result<(), String> {
    if !target_root.is_self_or_ancestor_of(final_canon) {
        return Err("aborted: handle escaped target root".to_string());
    }
    if final_canon.is_same_path(target_root) {
        if !(allow_delete_root && source_removes_root_after_contents(source)) {
            // Must not delete the target/custom/recycle-bin root itself.
            return Err("Skipped: target root must not be deleted".to_string());
        }
    } else if path_has_link_point_ancestor(final_canon, target_root, true)
        .map_err(|e| format!("Skipped: ancestor reparse check failed: {e}"))?
    {
        return Err("aborted: target root/ancestor link point detected".to_string());
    }
    // For the root-deletion case, ancestors up to (but excluding) the root were
    // checked in `ensure_target_root_safe`; the root itself must not be a link point.
    if final_canon.is_same_path(target_root)
        && path_has_link_point_ancestor(target_root, &root_parent_or_self(target_root), false)
            .map_err(|e| format!("Skipped: ancestor reparse check failed: {e}"))?
    {
        return Err("aborted: target root/ancestor link point detected".to_string());
    }
    Ok(())
}

/// Cloud/unknown reparse tags are never deleted; a Link Point is unlinked only
/// as the entry itself (no traversal).
fn reject_cloud_reparse(inspection: &HandleInspection) -> Result<(), String> {
    match inspection.reparse {
        ReparseTagClass::CloudPlaceholder | ReparseTagClass::OtherReparse(_) => {
            Err("Skipped: reparse point (cloud/unknown)".to_string())
        }
        ReparseTagClass::LinkPoint | ReparseTagClass::NotReparse => Ok(()),
    }
}

/// 4a) Identity binding: the entry behind the confirmed path must still be the
/// exact filesystem object observed when the item was validated. Child entries
/// of a root-style item are separate selections, so this applies only when
/// deleting the validated item itself. A required but missing identity fails
/// closed.
fn check_identity_binding(
    item: &ValidatedItem,
    requested_path: &str,
    inspection: &HandleInspection,
) -> Result<(), String> {
    if !item.identity_required {
        return Ok(());
    }
    if !paths_equal_str(requested_path, &item.path) {
        return Ok(());
    }
    if !identity_matches(item, inspection.identity) {
        return Err(FILE_CHANGED_SINCE_PREVIEW.to_string());
    }
    Ok(())
}

/// 5) Minimum Age check (exact boundary and future mtime treated as recent /
///    Skipped). Whole-cache sources are exempt; otherwise only directories emptied
///    this run are exempt, and the opened handle must still be a real directory.
fn check_minimum_age(
    item: &ValidatedItem,
    inspection: &HandleInspection,
    opts: &CleanOptions,
    ignore_emptied_dir_age: bool,
) -> Result<(), String> {
    if !age_check_applies(item, inspection, ignore_emptied_dir_age) {
        return Ok(());
    }
    let mtime_secs = filetime_to_unix_secs(inspection.last_write_time);
    let mtime = UNIX_EPOCH + Duration::from_secs(mtime_secs);
    if mtime > opts.now {
        return Err("Skipped: future mtime (treated as recent)".to_string());
    }
    match opts.now.duration_since(mtime) {
        Ok(age) if age < opts.min_age => Err("Skipped: newer than minimum age".to_string()),
        Ok(_) => Ok(()),
        Err(_) => Err("Skipped: future mtime (treated as recent)".to_string()),
    }
}

/// All inputs to [`revalidate_handle`] bundled into one context value.
///
/// The checks themselves are unchanged; grouping the six inputs keeps the
/// public helper at a single argument and avoids long positional argument
/// lists at every call site.
pub(super) struct RevalidateRequest<'a> {
    pub(super) item: &'a ValidatedItem,
    pub(super) requested_path: &'a str,
    pub(super) inspection: &'a HandleInspection,
    pub(super) opts: &'a CleanOptions,
    pub(super) allow_delete_root: bool,
    pub(super) ignore_emptied_dir_age: bool,
}

/// Moment-of-deletion re-validation on the opened handle.
pub(super) fn revalidate_handle(req: &RevalidateRequest<'_>) -> Result<(), String> {
    let final_path = strip_extended_prefix(&req.inspection.final_path);
    let final_canon = CanonicalPath::parse_lexical(&final_path)
        .map_err(|e| format!("Skipped: final path invalid: {e}"))?;
    let target_root = CanonicalPath::parse_lexical(&req.item.target_root)
        .map_err(|e| format!("Skipped: target root invalid: {e}"))?;

    reject_confirmed_path_redirect(&final_canon, req.requested_path, &target_root)?;
    // 1) Authorized source.
    ensure_authorized_source(&req.item.source);
    // 2) Protected Path exclusion.
    reject_protected_path(&final_canon)?;
    // 3) Final path containment and link-point ancestors.
    check_root_containment(
        &final_canon,
        &target_root,
        req.allow_delete_root,
        &req.item.source,
    )?;
    // 4) Reparse tag checks.
    reject_cloud_reparse(req.inspection)?;
    // 4a) Fail-closed identity binding.
    check_identity_binding(req.item, req.requested_path, req.inspection)?;
    // 5) Minimum Age.
    check_minimum_age(
        req.item,
        req.inspection,
        req.opts,
        req.ignore_emptied_dir_age,
    )
}

/// Open the item's path, inspect the handle, and fail closed unless the observed
/// identity still matches the preview identity.
///
/// This is the single open/inspect/compare sequence shared by
/// [`verify_root_anchor`] and [`revalidate_root_identity`]. Both callers keep the
/// same fail-closed rule and the exact error reasons; only their public return
/// values differ.
fn open_inspect_and_match_root_identity(item: &ValidatedItem) -> Result<SafeHandle, String> {
    let handle = sweeper_winfs::open_for_inspection(Path::new(&item.path))
        .map_err(|e| format!("Skipped: cannot inspect root identity: {e}"))?;
    let inspection = sweeper_winfs::inspect_handle(&handle)
        .map_err(|e| format!("Skipped: root identity inspection failed: {e}"))?;
    if !identity_matches(item, inspection.identity) {
        return Err(FILE_CHANGED_SINCE_PREVIEW.to_string());
    }
    Ok(handle)
}

/// Open and verify an anchor handle for a root-style item's directory.
///
/// The handle is held for the whole contents-clean batch. It gives the batch a
/// stable, handle-authoritative identity for the root: after a child is opened
/// we can re-check the root path against this handle (see
/// [`revalidate_batch_root`]) so a swap before the child open is caught and a
/// swap after it cannot redirect the already-open child. Identity verification
/// uses the same fail-closed rule and error strings as
/// [`revalidate_root_identity`]. `None` means no anchor could be obtained for
/// an unbound item, which keeps the original no-op behaviour for that case.
pub(super) fn verify_root_anchor(item: &ValidatedItem) -> Result<Option<SafeHandle>, String> {
    if !item.identity_required {
        return Ok(sweeper_winfs::open_for_inspection(Path::new(&item.path)).ok());
    }
    Ok(Some(open_inspect_and_match_root_identity(item)?))
}

/// Re-verify that the root path still resolves to the exact directory object
/// held in `anchor`.
///
/// This runs *after* a child handle is opened and before that handle is
/// deleted. A root swapped before the child open is detected here because the
/// path now resolves to a different identity; a swap after this check is
/// harmless because the child handle already refers to the right entry. This
/// closes the check-then-open window that a dropped root handle leaves open.
pub(super) fn revalidate_batch_root(
    item: &ValidatedItem,
    anchor: &SafeHandle,
) -> Result<(), String> {
    let expected = sweeper_winfs::get_file_identity(anchor)
        .map_err(|e| format!("Skipped: cannot inspect root identity: {e}"))?;
    let current = sweeper_winfs::open_for_inspection(Path::new(&item.path))
        .map_err(|e| format!("Skipped: cannot inspect root identity: {e}"))?;
    let observed = sweeper_winfs::get_file_identity(&current)
        .map_err(|e| format!("Skipped: root identity inspection failed: {e}"))?;
    if observed != expected {
        return Err(FILE_CHANGED_SINCE_PREVIEW.to_string());
    }
    Ok(())
}

/// Re-validate the bound filesystem identity of a root-style item before any of
/// its children are deleted.
///
/// Root-style selections (`WholeCache`, `AllowlistedTarget`, `CustomPath`, and
/// the Tier 3 roots) bind the identity of the root directory at preview time,
/// but `clean_directory_contents` deletes each child as a separate entry and
/// `revalidate_handle` only checks the binding when the entry being deleted is
/// the item path itself. Without this check, a root deleted and recreated
/// between Preview and Clean would have every file in the *new* directory
/// permanently deleted. Fails closed: a required-but-missing binding, an
/// unopenable/uncinspectable root, or any identity mismatch skips the whole
/// item with zero deletions.
pub(super) fn revalidate_root_identity(item: &ValidatedItem) -> Result<(), String> {
    if !item.identity_required {
        return Ok(());
    }
    open_inspect_and_match_root_identity(item).map(|_| ())
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
pub(super) fn reject_confirmed_path_redirect(
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

pub(super) fn ensure_target_root_safe(target_root: &str) -> Result<(), String> {
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

pub(super) fn path_has_link_point_ancestor(
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
