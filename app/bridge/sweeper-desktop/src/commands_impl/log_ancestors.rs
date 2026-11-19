//! Handle-based ancestor pinning for the log containment path.
//!
//! This module owns the pre-check, the root-to-leaf pinned handles, and the
//! bounded retry/contention classification used by `settings_logs::read_log`.
//! Keeping it separate makes the safety invariant explicit and keeps the
//! settings/log command module focused on command handling.

use std::fs;
use std::path::Path;

use sweeper_contract::ContractError;

use super::settings_logs::invalid_log_id;

/// Maximum attempts to acquire the ancestor pins. A sharing violation is often
/// transient (another process briefly holding a conflicting handle), so each
/// retry starts over from ancestor validation before trying the pin again.
const MAX_PIN_ATTEMPTS: usize = 3;

/// Outcome of the preliminary ancestor check.
#[derive(Debug)]
pub(super) enum AncestorCheck {
    /// No reparse-point ancestor was observed.
    Safe,
    /// A reparse point was seen, or an ancestor could not be inspected (fail closed).
    Unsafe,
    /// Another process holds a conflicting handle (ERROR_SHARING_VIOLATION).
    /// A valid log id must not be reported as invalid for this reason.
    Contention,
}

/// Why an ancestor pin could not be acquired.
#[derive(Debug)]
pub(super) enum PinError {
    /// A reparse point was seen, or an ancestor could not be opened/inspected.
    Unsafe,
    /// Another process holds a handle whose access conflicts with our
    /// restrictive share mode (ERROR_SHARING_VIOLATION).
    SharingViolation,
}

/// Result of one pin attempt. `Contention` is retryable; a valid id is never
/// mislabeled as invalid for a transient sharing violation.
enum PinAttempt {
    Pinned(PinnedAncestors),
    Contention,
}

/// RAII guard holding pins on every real ancestor of the logs directory from
/// the moment they are checked until the containment base and the log file have
/// been read.
///
/// Two independent TOCTOU gaps are closed here:
///
/// 1. `check_logs_path_ancestors` is a cheap preliminary check whose handles
///    are closed on return, so an attacker with write access to their own
///    `LOCALAPPDATA` tree could replace an ancestor with a junction before base
///    resolution. `FILE_FLAG_OPEN_REPARSE_POINT` suppresses reparse processing
///    only for the FINAL component, so base resolution would then follow that
///    junction and `GetFinalPathNameByHandle` would report the redirected target
///    as the containment base.
/// 2. Setting a reparse point needs WRITE access to the target directory, not
///    DELETE access. Denying only `FILE_SHARE_DELETE` would still let another
///    process open a pinned ancestor for write and convert it in place during
///    the pin window.
///
/// Ancestors are acquired ROOT-TO-LEAF: each component is opened only after its
/// parent is pinned, so every later absolute-path open resolves beneath a fixed,
/// already-pinned directory and no higher ancestor can be swapped between pins.
/// The volume root itself is an immutable traversal anchor (it cannot be renamed
/// or made a reparse point) and is therefore not pinned. Every handle is opened
/// with `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS` and
/// `FILE_SHARE_READ` only: read sharing is kept so ordinary reads and ancestor
/// traversal still work, while WRITE (needed to set a junction) and DELETE
/// (needed to rename/delete) sharing are denied. The reparse attribute is
/// re-checked on these very handles, so a chain of non-reparse ancestors pinned
/// root-to-leaf matches the path base resolution subsequently resolves.
///
/// 8.3 short names are ordinary directories, not reparse points, so genuine
/// short-name aliases remain accepted.
#[expect(
    dead_code,
    reason = "RAII guard: the handles are held open (denying WRITE/DELETE sharing) to pin ancestor identities; their contents are intentionally never read"
)]
pub(super) struct PinnedAncestors(Vec<fs::File>);

/// A distinct, retryable contention error. A sharing violation while pinning is
/// NOT an invalid log id: the ID can be perfectly valid while another process
/// momentarily holds a conflicting handle on an ordinary ancestor.
fn contention_error() -> ContractError {
    ContractError::Internal {
        reason: "logs directory is busy; retry the read".into(),
    }
}

/// Reject a configured logs path whose ANCESTORS include a reparse point
/// (junction, symlink, or mount point).
///
/// `FILE_FLAG_OPEN_REPARSE_POINT` suppresses reparse processing only for the
/// FINAL path component; ancestor junctions are still traversed when a handle is
/// opened, and `GetFinalPathNameByHandle` then reports the redirected target.
/// That would silently move the containment root away from the directory the
/// configured path actually names. 8.3 short names are NOT reparse points, so
/// genuine short-name segments remain allowed by this check. The logs directory
/// itself (the final component) is deliberately not examined here; case-variant
/// junction bases are handled by the exact-parent check in the caller.
///
/// A sharing violation is reported separately from an inspection failure so the
/// caller can classify it as retryable contention instead of an invalid log id.
pub(super) fn check_logs_path_ancestors(dir: &Path) -> AncestorCheck {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    let mut current = dir;
    while let Some(parent) = current.parent() {
        // `Path::parent()` on a drive root returns None, so every visited
        // prefix is a real directory to inspect.
        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent)
        {
            Ok(file) => file,
            Err(e) => {
                return if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) {
                    AncestorCheck::Contention
                } else {
                    AncestorCheck::Unsafe
                };
            }
        };
        let Ok(meta) = file.metadata() else {
            return AncestorCheck::Unsafe; // fail closed if an ancestor cannot be inspected
        };
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 || meta.is_symlink() {
            return AncestorCheck::Unsafe;
        }
        current = parent;
    }
    AncestorCheck::Safe
}

/// Open handles that pin every real ancestor of `dir` root-to-leaf.
pub(super) fn open_pinned_ancestors(dir: &Path) -> Result<PinnedAncestors, PinError> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    // `ancestors()` is leaf-first. Drop `dir` itself, then drop the volume root
    // (whose `parent()` is None) because it cannot be swapped, and reverse so
    // acquisition is root-to-leaf. Each open therefore traverses only parents
    // that are already pinned.
    let ancestors: Vec<&Path> = dir
        .ancestors()
        .skip(1)
        .take_while(|p| p.parent().is_some())
        .collect();
    let ancestors: Vec<&Path> = ancestors.into_iter().rev().collect();

    let mut handles = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors {
        let file = fs::OpenOptions::new()
            .read(true)
            // Keep READ sharing for reads/traversal; deny WRITE (needed to set a
            // junction) and DELETE (needed to rename/delete) sharing so a pinned
            // ancestor cannot be swapped or converted for the rest of the read.
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(ancestor)
            .map_err(|e| {
                if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) {
                    PinError::SharingViolation
                } else {
                    PinError::Unsafe
                }
            })?;
        let meta = file.metadata().map_err(|_| PinError::Unsafe)?;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 || meta.is_symlink() {
            return Err(PinError::Unsafe);
        }
        handles.push(file);
    }
    Ok(PinnedAncestors(handles))
}

/// Perform one pin attempt, restarting from ancestor validation.
///
/// Keeping the decision logic flat here lets the retry loop stay a simple
/// bounded `for` with a single conditional, avoiding nested branching.
fn attempt_pin(dir: &Path) -> Result<PinAttempt, ContractError> {
    match check_logs_path_ancestors(dir) {
        AncestorCheck::Safe => {}
        AncestorCheck::Unsafe => return Err(invalid_log_id()),
        AncestorCheck::Contention => return Ok(PinAttempt::Contention),
    }
    match open_pinned_ancestors(dir) {
        Ok(pinned) => Ok(PinAttempt::Pinned(pinned)),
        Err(PinError::Unsafe) => Err(invalid_log_id()),
        Err(PinError::SharingViolation) => Ok(PinAttempt::Contention),
    }
}

/// Acquire the authoritative ancestor pins, bounded-retrying a sharing violation.
///
/// Every attempt restarts from ancestor validation, so a reparse point swapped
/// in between attempts fails closed instead of being pinned through.
pub(super) fn pin_ancestors_with_retry(dir: &Path) -> Result<PinnedAncestors, ContractError> {
    for _ in 0..MAX_PIN_ATTEMPTS {
        if let PinAttempt::Pinned(pinned) = attempt_pin(dir)? {
            return Ok(pinned);
        }
    }
    Err(contention_error())
}
