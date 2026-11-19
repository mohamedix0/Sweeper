#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::path::Path;
use std::time::Duration;

use sweeper_core::journal;
use sweeper_core::remover::{self, CleanOptions, CleanReport};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

fn assert_child_gone(child: &Path, why: &str) {
    assert!(!child.exists(), "{why}");
}

fn assert_abort_reason_contains(report: &CleanReport, needle: &str) {
    assert!(
        report
            .abort_reason
            .as_deref()
            .unwrap_or("")
            .contains(needle),
        "abort reason: {:?}",
        report.abort_reason
    );
}

fn assert_single_deleted(report: &CleanReport, bytes: u64) {
    assert_eq!(report.items.len(), 1, "one item expected: {report:?}");
    assert_eq!(report.items[0].status, "deleted");
    assert_eq!(report.items[0].bytes_deleted, bytes);
}

#[test]
fn accounting_failed_and_unattempted_items_get_status_rows_after_journal_failure() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let a = target_root.join("a.txt");
    let b = target_root.join("b.txt");
    let c = target_root.join("c.txt");
    write_file(&a, b"delete A");
    write_file(&b, b"keep B");
    write_file(&c, b"keep C");
    for p in [&a, &b, &c] {
        set_filetime(p, old_mtime_secs());
    }
    let items = vec![
        make_item("item-a", &target_root, &a, 8),
        make_item("item-b", &target_root, &b, 6),
        make_item("item-c", &target_root, &c, 6),
    ];
    let journal_dir = sb.create_dir("journal").unwrap();

    // A's outcome write tears a record and its repair fails, poisoning the
    // journal. B's intent write then fails, stopping the batch before C runs.
    journal::fail_next_best_effort_outcome(journal::WriteFailure::AfterPrefix);
    journal::fail_outcome_repair_once();

    let report = remover::execute_clean(
        items,
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    assert_child_gone(&a, "A must have been deleted");
    assert!(
        b.exists(),
        "B must not be deleted after the journal became unusable"
    );
    assert!(c.exists(), "C is never attempted");

    let statuses: Vec<_> = report
        .items
        .iter()
        .map(|r| (r.id.as_str(), r.status.as_str()))
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("item-a", "deleted"),
            ("item-b", "Skipped"),
            ("item-c", "Skipped")
        ],
        "every selected item must have a status: {report:?}"
    );

    assert_eq!(report.skipped_count, 2);
    assert!(report.aborted);
    assert_sentinel_intact(&sb);
}

#[test]
fn accounting_completed_child_bytes_survive_circuit_breaker() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let child_a = target_root.join("child-a.bin");
    let child_b = target_root.join("child-b.bin");
    write_file(&child_a, &[b'a'; 9]);
    write_file(&child_b, &[b'b'; 5]);
    set_filetime(&child_a, old_mtime_secs());
    set_filetime(&child_b, old_mtime_secs());
    let dir_item = make_item("dir", &target_root, &target_root, 14);
    let journal_dir = sb.create_dir("journal").unwrap();

    // The first child's outcome write tears a record but its repair succeeds,
    // so the journal stays usable. The next child then trips the 120% breaker
    // (9 + 5 > 12). The already-deleted child's bytes must survive in the report.
    journal::fail_next_best_effort_outcome(journal::WriteFailure::AfterPrefix);

    let report = remover::execute_clean(
        vec![dir_item],
        CleanOptions {
            preview_estimated_bytes: 10, // 120% == 12 bytes
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    let (deleted_bytes, survivor) = if !child_a.exists() {
        (9u64, &child_b)
    } else {
        assert_child_gone(&child_b, "exactly one child must be deleted");
        (5u64, &child_a)
    };

    assert!(survivor.exists(), "breaker stops before the second child");
    assert!(report.aborted, "breaker must abort the clean");
    assert_abort_reason_contains(&report, "120%");

    assert_eq!(report.items.len(), 1, "one item expected: {report:?}");
    assert_eq!(
        report.items[0].status, "Skipped",
        "the abort decision is preserved"
    );
    assert_eq!(report.items[0].bytes_deleted, deleted_bytes);

    assert_eq!(report.bytes_freed, deleted_bytes);

    let errors = &report.journal_errors;
    assert!(
        !errors.is_empty(),
        "deferred outcome error must be surfaced"
    );
    assert_sentinel_intact(&sb);
}

#[test]
fn accounting_first_intent_failure_reports_no_deletion() {
    let sb = Sandbox::new();

    // Item A poisons the journal with a torn post-delete outcome write whose
    // repair fails, after its own deletion has completed.
    let file_root = sb.create_dir("file-root").unwrap();
    let file = file_root.join("a.txt");
    write_file(&file, b"aaaa");
    set_filetime(&file, old_mtime_secs());
    let item_a = make_item("item-a", &file_root, &file, 4);

    // Item B is a directory whose only child never gets an intent written.
    let dir_root = sb.create_dir("dir-root").unwrap();
    let child = dir_root.join("child.txt");
    write_file(&child, b"bbbb");
    set_filetime(&child, old_mtime_secs());
    let item_b = make_item("item-b", &dir_root, &dir_root, 4);

    let journal_dir = sb.create_dir("journal").unwrap();
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
    .unwrap();

    assert_child_gone(&file, "item A must have been deleted");
    assert!(child.exists(), "item B's child must remain untouched");

    let rows: Vec<_> = report.items.iter().filter(|r| r.id == "item-b").collect();
    assert_eq!(
        rows.len(),
        1,
        "item B must have exactly one status row: {report:?}"
    );
    assert_eq!(
        rows[0].status, "Skipped",
        "a failed first intent is not a deletion"
    );
    assert_eq!(rows[0].bytes_deleted, 0);

    assert!(report.aborted);
    assert_sentinel_intact(&sb);
}

#[test]
fn accounting_tier3_root_removal_failure_preserves_child_deletion() {
    let sb = Sandbox::new();
    let proj = sb.create_dir("proj").unwrap();
    let stale_dir = proj.join("target");
    fs::create_dir_all(&stale_dir).unwrap();
    let child = stale_dir.join("only-child.bin");
    write_file(&child, &[b'x'; 9]);
    set_filetime(&child, old_mtime_secs());
    set_filetime(&stale_dir, old_mtime_secs());

    let item = remover::ValidatedItem::from_tier3_project_directory(
        "stale-target",
        "stale-rust-target",
        canonical_str(&stale_dir),
        9,
        Some(old_mtime_secs()),
    )
    .unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();

    // The child's outcome write tears and its repair fails, poisoning the
    // journal. The now-empty root cannot be removed, but the child deletion
    // must still appear in the aborted report.
    journal::fail_next_best_effort_outcome(journal::WriteFailure::AfterPrefix);
    journal::fail_outcome_repair_once();

    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    assert_child_gone(&child, "the child must have been deleted");
    assert!(stale_dir.exists(), "root removal was expected to fail");

    assert_single_deleted(&report, 9);
    assert_eq!(report.bytes_freed, 9);

    assert!(report.aborted, "root removal failure stops the batch");
    assert_sentinel_intact(&sb);
}
