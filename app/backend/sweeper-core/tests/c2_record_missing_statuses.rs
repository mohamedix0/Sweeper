#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

//! Regression coverage for issue #132 (audit finding C2).
//!
//! A fatal journal failure mid-clean funnels every not-yet-processed item through
//! `record_missing_statuses`. That helper used a linear `report.items.iter().any(...)`
//! membership test against a vector it grows on every iteration, making the
//! journal-failure path quadratic in the batch size. This test drives that exact
//! path with a large batch and asserts both correct accounting and that runtime
//! scales roughly linearly with the batch size.

use std::time::{Duration, Instant};

use sweeper_core::journal::{self, WriteFailure};
use sweeper_core::remover::{self, CleanOptions, CleanReport, ValidatedItem, ValidatedSource};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

const SMALL_BATCH: usize = 1_000;
const LARGE_BATCH: usize = 8_000;
// Long ids make the string comparisons in the old O(n^2) path dominate fixed
// setup cost, so the scaling assertion is a reliable discriminator rather than
// a coin flip on a fast CI machine.
const ID_PAD_LEN: usize = 512;
const RUNS_PER_BATCH: usize = 3;

const ABS_RUNTIME_CAP: Duration = Duration::from_secs(10);
// 8x the items should cost roughly 8x the work with O(1) membership tests.
// The old per-item linear scan over `report.items` grows ~64x here. The slack
// covers fixed per-run overhead while still failing the quadratic path.
const SCALING_FACTOR_LIMIT: u32 = 12;

fn padded_id(index: usize) -> String {
    let pad = "x".repeat(ID_PAD_LEN);
    format!("miss-{index:05}-{pad}")
}

/// Build `count` items. Item 0 is a real file whose deletion poisons the
/// journal; the remaining items have valid but non-existent paths so they are
/// never opened — they exist only to be recorded as missing statuses.
fn build_items(sb: &Sandbox, count: usize) -> Vec<ValidatedItem> {
    let target_root = sb.create_dir("target").unwrap();
    let victim = target_root.join("poison.txt");
    write_file(&victim, b"delete me");
    set_filetime(&victim, old_mtime_secs());

    let root_str = canonical_str(&target_root);
    let mut items = vec![make_item("item-0-deleted", &target_root, &victim, 9)];
    for index in 1..count {
        let missing = target_root.join(format!("missing-{index}.tmp"));
        let path = path_str_for_item(&missing);
        items.push(remover::validated_item_for_tests(
            padded_id(index),
            "test-target",
            root_str.clone(),
            path,
            0,
            Some(old_mtime_secs()),
            ValidatedSource::CustomPath,
        ));
    }
    items
}

fn run_batch(sb: &Sandbox, items: Vec<ValidatedItem>) -> (CleanReport, Duration) {
    let journal_dir = sb.create_dir("journal").unwrap();
    // A's outcome write tears a record and its repair fails, poisoning the
    // journal. B's intent write then fails, stopping the batch and sending all
    // remaining items through `record_missing_statuses`.
    journal::fail_next_best_effort_outcome(WriteFailure::AfterPrefix);
    journal::fail_outcome_repair_once();

    let started = Instant::now();
    let report = remover::execute_clean(
        items,
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .expect("fatal journal failure must still return the honest partial report");
    (report, started.elapsed())
}

fn assert_partial_batch(sb: &Sandbox, report: &CleanReport, expected_total: usize) {
    assert!(report.aborted, "fatal journal failure must stop the batch");
    assert_eq!(
        report.items.len(),
        expected_total,
        "every item is accounted for"
    );
    assert_eq!(report.items[0].id, "item-0-deleted");
    assert_eq!(report.items[0].status, "deleted");
    assert_eq!(report.skipped_count, (expected_total - 1) as u64);

    let reason = report
        .abort_reason
        .as_deref()
        .expect("abort reason must be recorded");
    for (offset, row) in report.items[1..].iter().enumerate() {
        let index = offset + 1;
        assert_eq!(row.id, padded_id(index), "input order must be preserved");
        assert_eq!(row.status, "Skipped");
        assert_eq!(row.reason, reason);
        assert_eq!(row.bytes_deleted, 0);
    }
    assert_sentinel_intact(sb);
}

/// Best-of-N timing suppresses one-off scheduling/filesystem noise.
fn measure_best_of(count: usize) -> (CleanReport, Duration) {
    let mut best: Option<(CleanReport, Duration)> = None;
    for _ in 0..RUNS_PER_BATCH {
        let sb = Sandbox::new();
        let items = build_items(&sb, count);
        let (report, elapsed) = run_batch(&sb, items);
        assert_partial_batch(&sb, &report, count);
        if best.as_ref().is_none_or(|(_, prev)| elapsed < *prev) {
            best = Some((report, elapsed));
        }
    }
    best.expect("at least one run")
}

#[test]
fn c2_large_batch_journal_failure_is_correct_and_scales_linearly() {
    // Warm up filesystem/journal setup so cold caches do not skew the small run.
    let warm = Sandbox::new();
    let _ = run_batch(&warm, build_items(&warm, 8));

    let (_small_report, small_elapsed) = measure_best_of(SMALL_BATCH);
    let (_large_report, large_elapsed) = measure_best_of(LARGE_BATCH);

    eprintln!(
        "C2 timing: small({SMALL_BATCH})={small_elapsed:?} large({LARGE_BATCH})={large_elapsed:?} ratio={:.2}",
        large_elapsed.as_secs_f64() / small_elapsed.as_secs_f64().max(f64::EPSILON)
    );

    assert!(
        large_elapsed < ABS_RUNTIME_CAP,
        "large batch must finish within {ABS_RUNTIME_CAP:?}, took {large_elapsed:?}"
    );
    assert!(
        large_elapsed < small_elapsed.saturating_mul(SCALING_FACTOR_LIMIT),
        "runtime scaled super-linearly (old O(n^2) membership test reintroduced?): \
         small={SMALL_BATCH} in {small_elapsed:?}, large={LARGE_BATCH} in {large_elapsed:?}"
    );
}
