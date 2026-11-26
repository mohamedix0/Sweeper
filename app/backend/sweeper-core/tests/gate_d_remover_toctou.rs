#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::process::Command; // kept for other tests; TOCTOU hot loop uses fast Win32/testkit APIs
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, ValidatedSource};
use sweeper_testkit::{CanaryTree, Sandbox};
use sweeper_winfs::ReparseTagClass;

mod common;
use common::*;

/// Stops and joins the swap worker on any exit path, including a panic from
/// `execute_clean(...).unwrap()`. Dropping a `JoinHandle` would detach the
/// thread, leaving it mutating the sandbox while teardown runs.
struct WorkerGuard {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl WorkerGuard {
    fn new(stop: Arc<AtomicBool>, handle: thread::JoinHandle<()>) -> Self {
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn gate_d_toctou_swap_directory_for_junction_canaries_survive_1000_iterations() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let canary = CanaryTree::seed_external("gate_d_toctou_shared").unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();
    let old_ft = sweeper_testkit::filetime_from_unix_secs(old_mtime_secs());

    for iter in 0..1005 {
        let sub = target_root.join(format!("sub_{iter}"));

        // Best-effort cleanup from previous iteration artifacts without spawning cmd.exe.
        // If `sub` is a junction, remove_dir removes the reparse point only.
        let _ = fs::remove_file(sub.join("victim.txt"));
        let _ = fs::remove_dir(&sub);
        let _ = fs::remove_dir_all(&sub);
        fs::create_dir_all(&sub).unwrap();

        let victim = sub.join("victim.txt");
        write_file(&victim, format!("iter-{iter}").as_bytes());
        sweeper_testkit::set_last_write_time(&victim, old_ft).unwrap();
        sweeper_testkit::set_last_write_time(&sub, old_ft).unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let sub_bg = sub.clone();
        let canary_root = canary.root().to_path_buf();
        let victim_bg = sub_bg.join("victim.txt");
        let handle = thread::spawn(move || {
            while !stop_bg.load(Ordering::Relaxed) {
                // Real-dir state: ensure directory+file exist.
                let _ = fs::remove_dir(&sub_bg); // no-op unless it's a junction
                let _ = fs::create_dir_all(&sub_bg);
                let _ = fs::write(&victim_bg, b"x");

                // Swap to junction state pointing at external canary.
                let _ = fs::remove_file(&victim_bg);
                let _ = fs::remove_dir(&sub_bg);
                let _ = sweeper_testkit::create_junction(&sub_bg, &canary_root);
            }
        });
        // Join on both success and unwind: a panic in execute_clean must not
        // leave the worker detaching and mutating the sandbox during teardown.
        let worker = WorkerGuard::new(stop, handle);

        let item = make_item(&format!("i_{iter}"), &target_root, &sub, 1024);
        let _report = remover::execute_clean(
            vec![item],
            CleanOptions {
                preview_estimated_bytes: 1024 * 1024,
                min_age: Duration::from_secs(3600),
                now: now(),
                journal_base_dir: Some(journal_dir.clone()),
            },
        )
        .unwrap();

        drop(worker); // sets stop + joins before the canary check

        assert!(
            canary.verify().is_ok(),
            "external canary modified on iteration {iter}"
        );
    }
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_target_root_or_ancestor_swapped_to_junction_rejected_zero_deletions() {
    let sb = Sandbox::new();
    let real_root = sb.create_dir("real_target").unwrap();
    let victim = real_root.join("victim.txt");
    write_file(&victim, b"must survive");
    set_filetime(&victim, old_mtime_secs());

    let canary = CanaryTree::seed_external("gate_d_root_swap").unwrap();
    // Replace a path we will use as target root with junction to canary by swapping: remove empty dir case
    let target_root_alias = sb.root().join("target_alias");
    fs::create_dir_all(&target_root_alias).unwrap();
    // Swap alias to junction pointing outside (to canary)
    let _ = Command::new("cmd")
        .args([
            "/C",
            "rmdir",
            "/S",
            "/Q",
            &target_root_alias.to_string_lossy(),
        ])
        .status();
    sweeper_testkit::create_junction(&target_root_alias, canary.root()).unwrap();

    let alias_canon_lex =
        CanonicalPath::parse_lexical(&target_root_alias.to_string_lossy()).unwrap();
    // Construct item targeting alias (root) - but target root validation should detect link point/escape.
    let item = remover::validated_item_for_tests(
        "i1",
        "test-target",
        alias_canon_lex.as_str().to_string(),
        alias_canon_lex.as_str().to_string(), // root itself; remover will treat as contents-only root case
        0,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);
    // No deletions against canary / outside, and victim in real_root untouched
    assert!(victim.exists(), "victim must remain");
    assert!(canary.verify().is_ok(), "canary must remain intact");
    assert_eq!(
        report.bytes_freed, 0,
        "expected zero deletions, report={report:?}"
    );
    assert_sentinel_intact(&sb);
}

/// t1 regression: a required-but-missing identity binding must fail closed.
#[test]
fn gate_d_root_identity_missing_skips_all_children() {
    let sb = Sandbox::new();
    let journal_dir = sb.create_dir("journal").unwrap();

    let cache_root = sb.create_dir("cache_root").unwrap();
    let child = cache_root.join("victim.txt");
    write_file(&child, b"child victim");
    set_filetime(&child, old_mtime_secs());
    set_filetime(&cache_root, old_mtime_secs());

    let root_canon = canonical_str(&cache_root);
    let item = remover::validated_item_for_tests(
        "root-identity-missing",
        "test-target",
        root_canon.clone(),
        root_canon,
        0,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    )
    .with_identity_binding(None);

    let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    assert!(
        report
            .items
            .iter()
            .any(|r| r.reason.contains("Skipped: file changed since preview")),
        "missing identity must be reported: {report:?}"
    );
    assert_eq!(report.bytes_freed, 0, "no bytes may be deleted: {report:?}");
    assert_eq!(
        report.skipped_count, 1,
        "the whole root-style item must be skipped: {report:?}"
    );
    assert!(child.exists(), "child must survive a missing binding");
    assert_sentinel_intact(&sb);
}

/// subA1 coverage: the root-identity recheck applies to every root-style
/// selection, including sources that keep the root (`AllowlistedTarget`,
/// `CustomPath`) and not just the ones that unlink it (`WholeCache`).
#[test]
fn gate_d_root_identity_recheck_applies_to_all_root_sources() {
    let sb = Sandbox::new();
    let journal_dir = sb.create_dir("journal").unwrap();

    let other = sb.write_file("identity/source.txt", b"other").unwrap();
    let stale_identity =
        remover::capture_file_identity(&other).expect("other file must have a stable identity");

    let sources: Vec<(&str, ValidatedSource)> = vec![
        (
            "whole-cache",
            ValidatedSource::WholeCache {
                target_id: "test-target".to_string(),
            },
        ),
        (
            "allowlisted",
            ValidatedSource::AllowlistedTarget {
                target_id: "test-target".to_string(),
            },
        ),
        ("custom", ValidatedSource::CustomPath),
    ];

    for (label, source) in sources {
        let root = sb.create_dir(format!("cache_{label}")).unwrap();
        let child = root.join("victim.txt");
        let nested_child = root.join("nested").join("nested_victim.txt");
        write_file(&child, label.as_bytes());
        write_file(&nested_child, label.as_bytes());
        set_filetime(&child, old_mtime_secs());
        set_filetime(&nested_child, old_mtime_secs());
        set_filetime(&root, old_mtime_secs());

        let root_canon = canonical_str(&root);
        let item = remover::validated_item_for_tests(
            format!("id-{label}"),
            "test-target",
            root_canon.clone(),
            root_canon,
            0,
            Some(old_mtime_secs()),
            source,
        )
        .with_identity_binding(Some(stale_identity));

        let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

        assert!(
            report
                .items
                .iter()
                .any(|r| r.reason.contains("Skipped: file changed since preview")),
            "{label}: identity mismatch must be reported: {report:?}"
        );
        assert_eq!(
            report.bytes_freed, 0,
            "{label}: no bytes may be deleted: {report:?}"
        );
        assert_eq!(
            report.skipped_count, 1,
            "{label}: the whole root-style item must be skipped: {report:?}"
        );
        assert!(
            child.exists(),
            "{label}: child must survive identity mismatch"
        );
        assert!(
            nested_child.exists(),
            "{label}: nested child must survive identity mismatch"
        );
    }
    assert_sentinel_intact(&sb);
}

/// A root whose only file is newer than Minimum Age must report Skipped with
/// the preserved reason, not a fabricated `deleted` row with zero bytes.
#[test]
fn skipped_young_file_reports_skipped_reason_not_zero_byte_deletion() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("young_target").unwrap();
    let young = target_root.join("young.txt");
    write_file(&young, b"recent");
    // Deliberately leave the file at its current (recent) mtime. The item's
    // own mtime binding is old, but Minimum Age checks the opened child handle.

    let item = make_item("young-file", &target_root, &target_root, 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024 * 1024, Duration::from_secs(3600), &journal_dir);

    assert_eq!(report.items.len(), 1, "one item row expected: {report:?}");
    assert_eq!(
        report.items[0].status, "Skipped",
        "a root with only a skipped file must not be reported as deleted: {report:?}"
    );
    assert!(
        report.items[0].reason.contains("newer than minimum age"),
        "the skip reason must be preserved in the Clean result: {report:?}"
    );
    assert_eq!(report.items[0].bytes_deleted, 0);
    assert_eq!(report.skipped_count, 1);
    assert_eq!(report.bytes_freed, 0);
    assert!(
        young.exists(),
        "the newer-than-minimum-age file must survive"
    );
    assert_sentinel_intact(&sb);
}

/// A genuinely empty root has no skipped entry and must still finalize as a
/// zero-byte deletion (distinct from the skipped-file case above).
#[test]
fn empty_root_still_reports_deleted_with_zero_bytes() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("empty_target").unwrap();

    let item = make_item("empty-root", &target_root, &target_root, 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024 * 1024, Duration::from_secs(3600), &journal_dir);

    assert_eq!(report.items.len(), 1, "one item row expected: {report:?}");
    assert_eq!(
        report.items[0].status, "deleted",
        "an empty root must remain a zero-byte deletion: {report:?}"
    );
    assert_eq!(report.items[0].bytes_deleted, 0);
    assert_eq!(report.skipped_count, 0);
    assert!(
        target_root.exists(),
        "a CustomPath root is kept, not removed"
    );
    assert_sentinel_intact(&sb);
}

/// A child queued as an ordinary directory can be swapped for a Link Point
/// before its turn to be enumerated. The queued path must be inspected on a
/// reparse-safe handle first; when it is a Link Point the directory is Skipped
/// and never enumerated, so nothing behind it is deleted.
#[test]
fn queued_directory_swapped_to_link_point_is_skipped_not_traversed() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("swap_target").unwrap();
    let child = target_root.join("queued_child");
    std::fs::create_dir_all(&child).unwrap();
    let victim = child.join("victim.txt");
    write_file(&victim, b"must survive");
    set_filetime(&victim, old_mtime_secs());
    set_filetime(&child, old_mtime_secs());

    let canary = CanaryTree::seed_external("gate_d_queued_child_link").unwrap();

    // Enumeration of the parent sees an ordinary directory (the test hook only
    // changes handle inspection), so the child is queued. Inspection of that
    // queued directory then observes a Link Point, modelling a swap to a
    // junction that happened after enumeration and before traversal.
    sweeper_winfs::set_test_reparse_override(&child, Some(ReparseTagClass::LinkPoint));

    let item = make_item("queued-child", &target_root, &target_root, 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024 * 1024, Duration::from_secs(3600), &journal_dir);

    // Clear the global test hook before asserting so a failed assertion cannot
    // leak the override into other tests in this binary.
    sweeper_winfs::set_test_reparse_override(&child, None);

    assert!(
        victim.exists(),
        "the child must not be enumerated or deleted: {report:?}"
    );
    assert!(
        canary.verify().is_ok(),
        "canary behind the link must be intact"
    );
    assert_eq!(report.items.len(), 1, "one item row expected: {report:?}");
    assert_eq!(
        report.items[0].status, "Skipped",
        "the swapped queued directory must be reported as Skipped: {report:?}"
    );
    assert!(
        report.items[0].reason.contains("link point"),
        "the skip reason must name the Link Point: {report:?}"
    );
    assert_eq!(report.bytes_freed, 0);
    assert_sentinel_intact(&sb);
}
