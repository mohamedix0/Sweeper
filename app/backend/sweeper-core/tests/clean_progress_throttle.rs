#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::time::{Duration, Instant};

use sweeper_core::remover::{self, CleanOptions, CleanProgress};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

/// Mirrors the production clean-progress throttle (`CLEAN_PROGRESS_ITEM_THRESHOLD`).
const CLEAN_ITEM_THRESHOLD: u64 = 64;

/// Issue #131 (C1): per-item Clean progress must be throttled the same way scan
/// progress is (64 items / 100 ms), while the final update still carries exact
/// totals. Without the throttle every one of the items emits a callback.
#[test]
fn clean_progress_callbacks_are_bounded_and_final_totals_stay_accurate() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();

    // Deliberately not a multiple of 64 so the last item cannot ride a regular
    // 64-item emit — the terminal update must be forced to reach the caller.
    const FILE_COUNT: u64 = 2_000;
    let mut items = Vec::with_capacity(FILE_COUNT as usize);
    for i in 0..FILE_COUNT {
        let path = target_root.join(format!("file-{i:05}.tmp"));
        write_file(&path, b"x");
        set_filetime(&path, old_mtime_secs());
        items.push(make_item(&format!("item-{i}"), &target_root, &path, 1));
    }

    let mut callbacks: Vec<CleanProgress> = Vec::new();
    let started = Instant::now();
    let report = remover::execute_clean_with_progress(
        items,
        CleanOptions {
            preview_estimated_bytes: FILE_COUNT * 4, // comfortably above 120% of 2000 bytes
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
        |progress| callbacks.push(progress),
    )
    .unwrap();
    let elapsed = started.elapsed();

    assert!(
        !report.aborted,
        "clean must not abort: {:?}",
        report.abort_reason
    );
    assert_eq!(report.bytes_freed, FILE_COUNT, "every file must be deleted");

    // Between two emits at least one of the throttle limits must be reached:
    // 64 items or 100 ms. Bound the expected count by both, with slack for the
    // terminal update. Unthrottled code emits once per item (FILE_COUNT times).
    let bound =
        (elapsed.as_millis() / 100) as usize + (FILE_COUNT / CLEAN_ITEM_THRESHOLD) as usize + 8;
    assert!(
        callbacks.len() <= bound,
        "progress callbacks were not throttled: {} emitted for {FILE_COUNT} items in {elapsed:?} (bound {bound})",
        callbacks.len()
    );

    // The final, forced update always carries the exact totals.
    let last = callbacks
        .last()
        .expect("at least one progress event must be emitted");
    assert_eq!(last.completed_items, FILE_COUNT);
    assert_eq!(last.total_items, FILE_COUNT);
    assert_eq!(last.bytes_freed_so_far, FILE_COUNT);
    assert_sentinel_intact(&sb);
}
