//! Whole-cache-specific Clean helpers (issue #41).
//!
//! A small dedicated module keeps `remover.rs` within its module-size limit
//! while holding the two helpers that encode whole-cache outcome and age rules.

use sweeper_winfs::{HandleInspection, ReparseTagClass};

use super::{inspection_is_directory, ProcessOutcome, ValidatedItem, ValidatedSource};

/// Outcome for a whole-cache clean whose root still holds entries after the
/// content sweep. The cache was not fully removed, so it is reported Skipped,
/// but the bytes already deleted are carried into the report.
pub(super) fn whole_cache_partial_outcome(bytes: u64) -> ProcessOutcome {
    ProcessOutcome::Skipped(
        "Skipped: whole-cache not fully cleaned (entries remain in cache root)".to_string(),
        bytes,
    )
}

/// Whether Minimum Age must be enforced for this opened handle.
///
/// Issue #41 whole-cache sources never filter by age (the cache is cleaned as
/// one unit). Otherwise the only exemption is a directory we actually emptied
/// during this run, provided it is still a real directory on the opened handle
/// (TOCTOU replaced-by-file/reparse entries still get the full age check).
pub(super) fn age_check_applies(
    item: &ValidatedItem,
    inspection: &HandleInspection,
    ignore_emptied_dir_age: bool,
) -> bool {
    if matches!(item.source, ValidatedSource::WholeCache { .. }) {
        return false;
    }
    // Pre-existing carve-out: a directory actually emptied during this run had
    // its LastWriteTime bumped by our own deletions, so its age is not meaningful.
    // Every other ordinary item MUST still be checked against Minimum Age.
    let emptied_directory_carve_out = ignore_emptied_dir_age
        && inspection_is_directory(inspection)
        && inspection.reparse_tag == 0
        && matches!(inspection.reparse, ReparseTagClass::NotReparse);
    !emptied_directory_carve_out
}
