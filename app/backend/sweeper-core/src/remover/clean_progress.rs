//! Throttled per-item progress emission for Clean runs (issue #131).
//!
//! Kept in a focused module so the safety-critical remover stays small. The
//! limits mirror the scan progress path (`scanner::runtime::ScanProgressRuntime`).

use std::time::{Duration, Instant};

use super::{CleanProgress, ValidatedItem};

const CLEAN_PROGRESS_ITEM_THRESHOLD: u64 = 64;
const CLEAN_PROGRESS_TIME_THRESHOLD: Duration = Duration::from_millis(100);

/// Rate-limits per-item Clean progress to at most one emit per 64 items, or one
/// per 100 ms. The state is per-run and lives on the stack, so concurrent Clean
/// runs never share throttle state.
pub(super) struct CleanProgressThrottle {
    last_emit_at: Instant,
    last_emit_items: u64,
}

impl CleanProgressThrottle {
    pub(super) fn new() -> Self {
        Self {
            last_emit_at: Instant::now(),
            last_emit_items: 0,
        }
    }

    fn emit<F>(&mut self, on_progress: &mut F, progress: CleanProgress)
    where
        F: FnMut(CleanProgress),
    {
        self.last_emit_at = Instant::now();
        self.last_emit_items = progress.completed_items;
        on_progress(progress);
    }

    /// Emit only when a throttle limit is reached. Each limit is a separate
    /// early return so the decision stays simple to audit (CodeScene C1 follow-up).
    pub(super) fn maybe_emit<F>(
        &mut self,
        on_progress: &mut F,
        progress: CleanProgress,
        force: bool,
    ) where
        F: FnMut(CleanProgress),
    {
        if force {
            self.emit(on_progress, progress);
            return;
        }

        let items_delta = progress
            .completed_items
            .saturating_sub(self.last_emit_items);
        if items_delta >= CLEAN_PROGRESS_ITEM_THRESHOLD {
            self.emit(on_progress, progress);
            return;
        }

        if self.last_emit_at.elapsed() >= CLEAN_PROGRESS_TIME_THRESHOLD {
            self.emit(on_progress, progress);
        }
    }
}

/// Emit a progress update for a single processed item, throttled to at most one
/// event per 64 items or 100 ms. Terminal updates (the last item of the run or
/// an abort) bypass the throttle so final totals always reach the caller.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_item_progress<F>(
    throttle: &mut CleanProgressThrottle,
    on_progress: &mut F,
    item: &ValidatedItem,
    completed: u64,
    total_items: u64,
    bytes_freed: u64,
    status: String,
) where
    F: FnMut(CleanProgress),
{
    let terminal = status == "aborted" || completed == total_items;
    throttle.maybe_emit(
        on_progress,
        CleanProgress {
            target_id: item.target_id.clone(),
            completed_items: completed,
            total_items,
            bytes_freed_so_far: bytes_freed,
            status,
        },
        terminal,
    );
}
