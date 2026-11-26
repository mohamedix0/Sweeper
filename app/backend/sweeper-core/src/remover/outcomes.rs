//! Journal and outcome helpers for `remover`.
//!
//! Report construction, per-item outcome application and finalization after a
//! fatal journal failure live here so the parent module only orchestrates the
//! batch. All reason strings, journal records and `ProcessOutcome` semantics
//! are unchanged from the original inline implementation.

use std::collections::HashSet;

use crate::journal::{self, JournalWriter};

use super::clean_progress::{emit_item_progress, CleanProgressThrottle, ItemProgress};
use super::{
    CleanError, CleanProgress, CleanReport, ItemCleanResult, ValidatedItem, ValidatedSource,
};

/// Per-item processing outcome produced by the deletion chain.
pub(super) enum ProcessOutcome {
    Deleted(u64),
    /// Skipped with a reason and the bytes already deleted by a partial traversal.
    Skipped(String, u64),
    /// Aborted with a reason and the bytes already deleted by a partial traversal.
    Aborted(String, u64),
}

impl ProcessOutcome {
    /// A skip outcome with no completed deletions.
    pub(super) fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped(reason.into(), 0)
    }

    /// An abort outcome with no completed deletions.
    pub(super) fn aborted(reason: impl Into<String>) -> Self {
        Self::Aborted(reason.into(), 0)
    }
}

/// Build a fresh, empty report for a Clean run of `item_count` items.
pub(super) fn new_clean_report(item_count: usize) -> CleanReport {
    CleanReport {
        items: Vec::with_capacity(item_count),
        bytes_freed: 0,
        skipped_count: 0,
        aborted: false,
        abort_reason: None,
        journal_errors: Vec::new(),
    }
}

/// Stop a clean batch after a fatal journal failure while preserving the
/// deletions that already completed and every journal error collected so far.
pub(super) fn stop_after_journal_failure(
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
pub(super) fn record_missing_statuses(
    report: &mut CleanReport,
    items: &[ValidatedItem],
    reason: &str,
) {
    // Membership is O(1) via a set built once from the rows already recorded;
    // the old per-item linear scan over the growing `report.items` was O(n^2).
    // Insert-while-iterating also mirrors the old behaviour for duplicate ids.
    let mut recorded: HashSet<&str> = report.items.iter().map(|row| row.id.as_str()).collect();
    let mut additions = Vec::with_capacity(items.len());
    for item in items {
        if !recorded.insert(item.id.as_str()) {
            continue;
        }
        additions.push(ItemCleanResult {
            id: item.id.clone(),
            path: item.path.clone(),
            status: "Skipped".to_string(),
            reason: reason.to_string(),
            bytes_deleted: 0,
        });
        report.skipped_count += 1;
    }
    report.items.extend(additions);
}

/// Named inputs for a post-abort skip. This is the single named-field argument
/// of [`record_post_abort_skip`]; journal writes, report accounting and
/// progress emission are unchanged from the original positional form.
pub(super) struct PostAbortSkipContext<'a, F> {
    pub(super) journal: &'a mut JournalWriter,
    pub(super) report: &'a mut CleanReport,
    pub(super) item: &'a ValidatedItem,
    pub(super) completed: &'a mut u64,
    pub(super) total_items: u64,
    pub(super) throttle: &'a mut CleanProgressThrottle,
    pub(super) on_progress: &'a mut F,
}

/// Record the skip of an item not processed because the batch already aborted,
/// then emit its (throttled) progress update.
pub(super) fn record_post_abort_skip<F>(ctx: PostAbortSkipContext<'_, F>) -> Result<(), CleanError>
where
    F: FnMut(CleanProgress),
{
    let PostAbortSkipContext {
        journal,
        report,
        item,
        completed,
        total_items,
        throttle,
        on_progress,
    } = ctx;
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
pub(super) fn apply_process_outcome(
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
pub(super) fn source_removes_root_after_contents(source: &ValidatedSource) -> bool {
    matches!(
        source,
        ValidatedSource::Tier3ProjectDirectory { .. } | ValidatedSource::WholeCache { .. }
    )
}

/// Fold completed child bytes into a Tier 3 root outcome that did not delete
/// the root itself. A successful `Deleted` already accounts for them; a
/// `Skipped` or `Aborted` root must not drop the work done on its children.
pub(super) fn root_outcome_with_child_bytes(
    outcome: ProcessOutcome,
    child_bytes: u64,
) -> ProcessOutcome {
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

/// Journal a successful deletion for one entry.
///
/// Issue #135: only this entry's own bytes are journaled. For a Tier 3 /
/// WholeCache root, child bytes are already recorded by the per-child outcomes,
/// so repeating them here double-counts the log/summary path. The caller still
/// returns the full total to `CleanReport`, whose accounting is unchanged.
pub(super) fn journal_deleted_outcome(
    journal: &mut JournalWriter,
    target_id: &str,
    path_str: &str,
    bytes: u64,
) {
    // The deletion already succeeded; a journal failure must not abort the batch.
    journal.write_outcome_best_effort(&journal::outcome_record(
        target_id, path_str, bytes, "deleted", "", None,
    ));
}

/// Journal an intent plus a Skipped outcome for an item/finalization row.
pub(super) fn record_skip(
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
