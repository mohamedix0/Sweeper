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
