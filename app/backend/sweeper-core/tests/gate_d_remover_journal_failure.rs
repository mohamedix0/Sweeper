#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::time::Duration;

use sweeper_core::journal;
use sweeper_core::remover::{self, CleanOptions, CleanReport};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

// Small assertion helpers keep contiguous assertion blocks below CodeScene's
// threshold of four without dropping any behavioural coverage.

fn assert_single_deleted_item(report: &CleanReport, id: &str, bytes: u64) {
    assert_eq!(report.items.len(), 1, "partial report must keep one item");
    assert_eq!(report.items[0].id, id);
    assert_eq!(report.bytes_freed, bytes);
}

fn assert_journal_error_contains(report: &CleanReport, needle: &str, why: &str) {
    assert!(
        report.journal_errors.iter().any(|e| e.contains(needle)),
        "{why}: {:?}",
        report.journal_errors
    );
}

fn assert_exactly_one_child_gone(a_gone: bool, b_gone: bool) {
    assert!(
        a_gone ^ b_gone,
        "exactly one child must be deleted: a={a_gone}, b={b_gone}"
    );
}

#[test]
fn gate_d_post_delete_journal_failure_returns_honest_partial_report() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let victim = target_root.join("journal-failure.txt");
    write_file(&victim, b"delete me");
    set_filetime(&victim, old_mtime_secs());
    let item = make_item("post-delete-journal", &target_root, &victim, 9);
    let journal_dir = sb.create_dir("journal").unwrap();

    // Simulate a journal write failure *after* the handle-based deletion succeeds.
    journal::fail_next_best_effort_outcome(journal::WriteFailure::BeforeWrite);
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    // The deletion actually happened, and the honest report keeps that outcome.
    assert!(!victim.exists(), "eligible file must have been deleted");
    assert_single_deleted_item(&report, "post-delete-journal", 9);
    assert_eq!(report.items[0].status, "deleted");
    assert!(!report.aborted, "journal failure is not a safety abort");
    assert!(
        !report.journal_errors.is_empty(),
        "post-delete journal failure must be recorded and surfaced"
    );
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_persistent_outcome_repair_failure_stops_batch_with_partial_report() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let victim_a = target_root.join("a.txt");
    let victim_b = target_root.join("b.txt");
    write_file(&victim_a, b"delete A");
    write_file(&victim_b, b"keep B");
    set_filetime(&victim_a, old_mtime_secs());
    set_filetime(&victim_b, old_mtime_secs());
    let item_a = make_item("item-a", &target_root, &victim_a, 8);
    let item_b = make_item("item-b", &target_root, &victim_b, 6);
    let journal_dir = sb.create_dir("journal").unwrap();

    // A's outcome write tears a record and its repair fails, so the journal is
    // persistently unusable. B's intent write then fails naturally (no one-shot
    // injection needed) and must stop the batch while keeping A's result.
    journal::fail_next_best_effort_outcome(journal::WriteFailure::AfterPrefix);
    journal::fail_outcome_repair_once();

    let report = remover::execute_clean(
        vec![item_a, item_b],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .expect("persistent journal failure must still return the honest partial report");

    assert!(!victim_a.exists(), "A must have been deleted");
    assert!(
        victim_b.exists(),
        "B must not be deleted once the journal became unusable"
    );

    let rows: Vec<_> = report
        .items
        .iter()
        .map(|r| (r.id.as_str(), r.status.as_str()))
        .collect();
    assert_eq!(rows, vec![("item-a", "deleted"), ("item-b", "Skipped")]);

    assert_journal_error_contains(
        &report,
        "repair failed",
        "the persistent outcome/repair error must be surfaced",
    );

    assert_journal_error_contains(
        &report,
        "journal unavailable after a failed repair",
        "the fatal intent error must be surfaced",
    );
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_directory_child_deletion_partial_result_survives_next_child_intent_failure() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let child_a = target_root.join("child-a.txt");
    let child_b = target_root.join("child-b.txt");
    write_file(&child_a, b"aaaaaaaaa");
    write_file(&child_b, b"bbbbbbbbb");
    set_filetime(&child_a, old_mtime_secs());
    set_filetime(&child_b, old_mtime_secs());
    // A custom-path root item exercises contents-only directory cleaning.
    let dir_item = make_item("dir", &target_root, &target_root, 18);
    let journal_dir = sb.create_dir("journal").unwrap();

    // The first child's outcome write tears a record and its repair fails,
    // poisoning the journal. The second child's intent write then fails and
    // stops the batch, but the first child's deletion must still be reported.
    journal::fail_next_best_effort_outcome(journal::WriteFailure::AfterPrefix);
    journal::fail_outcome_repair_once();

    let report = remover::execute_clean(
        vec![dir_item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .expect("journal failure must still return the honest partial report");

    let first_gone = !child_a.exists();
    let second_gone = !child_b.exists();
    assert_exactly_one_child_gone(first_gone, second_gone);
    assert_single_deleted_item(&report, "dir", 9);
    assert_eq!(report.items[0].status, "deleted");
    assert!(
        report.aborted,
        "a fatal journal failure must stop the batch"
    );
    assert_journal_error_contains(
        &report,
        "repair failed",
        "the deferred repair error must be surfaced",
    );
    assert_journal_error_contains(
        &report,
        "journal unavailable after a failed repair",
        "the fatal intent error must be surfaced",
    );
    assert_sentinel_intact(&sb);
}
