#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::path::Path;
use std::process::Command; // kept for other tests; TOCTOU hot loop uses fast Win32/testkit APIs
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, CleanReport, ValidatedSource};
use sweeper_testkit::{CanaryTree, Sandbox};
use sweeper_winfs::ReparseTagClass;

mod common;
use common::*;

/// Asserts that the clean report contains exactly one item row whose status and
/// reason match the expected values.
fn assert_single_item_status_and_reason(
    report: &CleanReport,
    context: &str,
    status: &str,
    reason_fragment: &str,
) {
    assert_eq!(
        report.items.len(),
        1,
        "{context}: one item row expected: {report:?}"
    );
    assert_eq!(
        report.items[0].status, status,
        "{context}: unexpected item status: {report:?}"
    );
    assert!(
        report.items[0].reason.contains(reason_fragment),
        "{context}: item reason must mention {reason_fragment:?}: {report:?}"
    );
}

/// Asserts that the sole item row accounts for zero deleted bytes.
fn assert_single_item_zero_bytes(report: &CleanReport, context: &str) {
    assert_eq!(
        report.items[0].bytes_deleted, 0,
        "{context}: item must account for zero deleted bytes: {report:?}"
    );
}

/// Asserts that an empty root is finalized as exactly one zero-byte deletion.
fn assert_single_item_deleted_zero_bytes(report: &CleanReport, context: &str) {
    assert_eq!(
        report.items.len(),
        1,
        "{context}: one item row expected: {report:?}"
    );
    assert_eq!(
        report.items[0].status, "deleted",
        "{context}: an empty root must remain a zero-byte deletion: {report:?}"
    );
    assert_eq!(
        report.items[0].bytes_deleted, 0,
        "{context}: empty root must delete zero bytes: {report:?}"
    );
}

/// Asserts the report-level byte and skip totals.
fn assert_clean_totals(report: &CleanReport, context: &str, bytes_freed: u64, skipped_count: u64) {
    assert_eq!(
        report.bytes_freed, bytes_freed,
        "{context}: unexpected byte total: {report:?}"
    );
    assert_eq!(
        report.skipped_count, skipped_count,
        "{context}: unexpected skip count: {report:?}"
    );
}

/// Asserts that no bytes were freed (without assuming a particular skip count).
fn assert_no_bytes_freed(report: &CleanReport, context: &str) {
    assert_eq!(
        report.bytes_freed, 0,
        "{context}: no bytes may be deleted: {report:?}"
    );
}

/// Asserts that a root-style item was skipped because its identity binding was
/// missing or stale, and that the whole item was skipped.
fn assert_identity_mismatch_skipped(report: &CleanReport, context: &str) {
    assert!(
        report
            .items
            .iter()
            .any(|r| r.reason.contains("Skipped: file changed since preview")),
        "{context}: missing/stale identity must be reported: {report:?}"
    );
    assert_eq!(
        report.skipped_count, 1,
        "{context}: the whole root-style item must be skipped: {report:?}"
    );
}

/// Asserts that each of the given victim paths still exists.
fn assert_paths_survive<P: AsRef<Path>>(paths: &[P], context: &str) {
    for path in paths {
        let path = path.as_ref();
        assert!(
            path.exists(),
            "{context}: victim must survive: {}",
            path.display()
        );
    }
}

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

    assert_identity_mismatch_skipped(&report, "root-identity-missing");
    assert_no_bytes_freed(&report, "root-identity-missing");
    assert_paths_survive(&[&child], "root-identity-missing");
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

        assert_identity_mismatch_skipped(&report, label);
        assert_no_bytes_freed(&report, label);
        assert_paths_survive(&[&child, &nested_child], label);
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

    assert_single_item_status_and_reason(
        &report,
        "skipped-young-file",
        "Skipped",
        "newer than minimum age",
    );
    assert_single_item_zero_bytes(&report, "skipped-young-file");
    assert_clean_totals(&report, "skipped-young-file", 0, 1);
    assert_paths_survive(&[&young], "skipped-young-file");
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

    assert_single_item_deleted_zero_bytes(&report, "empty-root");
    assert_clean_totals(&report, "empty-root", 0, 0);
    assert_paths_survive(&[&target_root], "empty-root");
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

    assert_paths_survive(&[&victim], "queued-child");
    assert!(
        canary.verify().is_ok(),
        "canary behind the link must be intact"
    );
    assert_single_item_status_and_reason(&report, "queued-child", "Skipped", "link point");
    assert_no_bytes_freed(&report, "queued-child");
    assert_sentinel_intact(&sb);
}

/// Asserts that the external canary manifest is unchanged after the clean.
fn assert_canary_intact(canary: &CanaryTree, context: &str) {
    assert!(
        canary.verify().is_ok(),
        "{context}: external canary must remain intact"
    );
}

/// Asserts that no bytes outside the known in-sandbox files were accounted as freed.
fn assert_no_external_bytes_freed(report: &CleanReport, context: &str, local_budget: u64) {
    assert!(
        report.bytes_freed <= local_budget,
        "{context}: only in-sandbox local bytes may be freed: {report:?}"
    );
}

/// Concatenates the clean journal records so a test can check which paths were
/// touched without depending on the per-run journal file name.
fn read_clean_journal_text(journal_dir: &Path) -> String {
    let mut all = String::new();
    if let Ok(entries) = fs::read_dir(journal_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("clean-") && name.ends_with(".jsonl") {
                if let Ok(text) = fs::read_to_string(entry.path()) {
                    all.push_str(&text);
                }
            }
        }
    }
    all
}

/// Asserts that the queued child was handled inside the sandbox, that no
/// journal path pointed into the canary, and that any abort was the safety
/// guard stopping a link-point traversal (not an escape that freed canary data).
fn assert_queued_path_safe(report: &CleanReport, journal_dir: &Path, child: &Path, context: &str) {
    let text = read_clean_journal_text(journal_dir);
    let child_name = child.file_name().unwrap().to_string_lossy().to_string();
    let child_handled = text.contains(&child_name) || !child.join("victim.txt").exists();
    let no_canary_target = !text.to_ascii_lowercase().contains("canary");
    let safe_abort = !report.aborted
        || report
            .abort_reason
            .as_deref()
            .map(|r| r.contains("link point") || r.contains("divergence"))
            .unwrap_or(false);
    assert!(
        child_handled && no_canary_target && safe_abort,
        "{context}: queued child must be skipped/untouched with no canary target: {report:?}"
    );
}

/// Regression for the race called out on PR #190: an ordinary child queued for
/// bottom-up traversal is swapped for a REAL junction to an external canary
/// while the clean runs. The enumeration anchor denies delete share so the
/// entry cannot be replaced between the reparse check and the name-based read,
/// and a child already swapped to a link point must be Skipped on its handle.
/// No bytes behind the junction may be freed and the canary must stay intact.
#[test]
fn gate_d_queued_child_real_junction_swap_cannot_free_external_bytes() {
    const LOCAL_BYTES: &[u8] = b"local-old";
    const VICTIM_BYTES: &[u8] = b"child-victim";
    let local_budget = (LOCAL_BYTES.len() + VICTIM_BYTES.len()) as u64;

    let sb = Sandbox::new();
    let target_root = sb.create_dir("real_swap_root").unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();

    // An old, legitimately deletable local file in the contents root.
    let local = target_root.join("local_old.txt");
    write_file(&local, LOCAL_BYTES);
    set_filetime(&local, old_mtime_secs());
    set_filetime(&target_root, old_mtime_secs());

    // A queued child directory holding an old local victim.
    let child = target_root.join("queued_child");
    fs::create_dir_all(&child).unwrap();
    let victim = child.join("victim.txt");
    write_file(&victim, VICTIM_BYTES);
    set_filetime(&victim, old_mtime_secs());
    set_filetime(&child, old_mtime_secs());

    // External canary made "old" so a wrong traversal would actually delete it.
    let canary = CanaryTree::seed_external("gate_d_queued_child_real_swap").unwrap();
    for rel in ["root.txt", "nested/file.txt", "nested/data.bin"] {
        set_filetime(&canary.root().join(rel), old_mtime_secs());
    }

    // Background worker alternates the queued child between an ordinary
    // directory and a real junction to the canary. Only `remove_dir` (never
    // `remove_dir_all`) is used on the link path so the worker itself can never
    // delete canary contents.
    let stop = Arc::new(AtomicBool::new(false));
    let stop_bg = stop.clone();
    let child_bg = child.clone();
    let canary_root = canary.root().to_path_buf();
    let old_ft = sweeper_testkit::filetime_from_unix_secs(old_mtime_secs());
    let handle = thread::spawn(move || {
        while !stop_bg.load(Ordering::Relaxed) {
            let victim_bg = child_bg.join("victim.txt");
            // Restore ordinary-directory state with an old local victim.
            let _ = fs::remove_file(&victim_bg);
            let _ = fs::remove_dir(&child_bg);
            let _ = fs::create_dir_all(&child_bg);
            let _ = fs::write(&victim_bg, VICTIM_BYTES);
            let _ = sweeper_testkit::set_last_write_time(&victim_bg, old_ft);

            // Swap the queued entry to a junction pointing at the canary.
            let _ = fs::remove_file(&victim_bg);
            let _ = fs::remove_dir(&child_bg);
            let _ = sweeper_testkit::create_junction(&child_bg, &canary_root);
        }
    });
    let worker = WorkerGuard::new(stop, handle);

    let item = make_item("queued-real-swap", &target_root, &target_root, 1024);
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(3600),
            now: now(),
            journal_base_dir: Some(journal_dir.clone()),
        },
    )
    .unwrap();

    // Stop and join the worker before any integrity assertion.
    drop(worker);

    assert_canary_intact(&canary, "queued-real-swap");
    assert_no_external_bytes_freed(&report, "queued-real-swap", local_budget);
    assert_queued_path_safe(&report, &journal_dir, &child, "queued-real-swap");
    assert_sentinel_intact(&sb);
}
