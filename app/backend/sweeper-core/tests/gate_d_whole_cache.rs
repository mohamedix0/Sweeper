//! Issue #41 — whole-cache `Clean` semantics inside the sentinel sandbox.
//!
//! A whole-cache Target is removed as one unit: its entire directory root is
//! deleted after its contents, with no per-file Minimum Age filtering. Canaries
//! outside the Target must survive, and protected/shallow roots must be rejected.

#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup.

use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::{CanaryTree, Sandbox};

mod common;
use common::*;

#[test]
fn gate_d_whole_cache_removes_entire_root_bypasses_age_and_preserves_canary() {
    let sb = Sandbox::new();
    let cache_root = sb.create_dir("uv-cache").unwrap();
    let nested = cache_root.join("v1").join("archive");
    write_file(&nested.join("a.bin"), b"aaaa");
    write_file(&cache_root.join("v1").join("b.bin"), b"bb");

    // Deliberately fresh mtimes: whole-cache cleaning must NOT apply Minimum Age.
    let now_secs = unix_now();
    for p in [&nested.join("a.bin"), &cache_root.join("v1").join("b.bin")] {
        set_filetime(p, now_secs);
    }

    // A separate, non-targeted directory in the same sandbox must survive.
    let bystander = sb.create_dir("bystander").unwrap();
    write_file(&bystander.join("keep.txt"), b"keep");

    let canary = CanaryTree::seed_external("gate_d_whole_cache").unwrap();
    let item = remover::ValidatedItem::from_whole_cache_root(
        "wc-uv-1",
        "uv-cache",
        canonical_str(&cache_root),
        6,
    )
    .expect("whole-cache root must validate");
    let journal_dir = sb.create_dir("journal").unwrap();
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

    assert!(!report.aborted, "aborted: {:?}", report.abort_reason);
    assert!(
        !cache_root.exists(),
        "whole-cache root must be removed: {:?}",
        report.items
    );
    assert!(
        bystander.join("keep.txt").exists(),
        "bystander must survive"
    );
    assert!(canary.verify().is_ok(), "canary must remain intact");
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_whole_cache_rejects_protected_and_shallow_roots() {
    let protected = r"C:\Users\Alice\Documents\uv-cache";
    let shallow = r"C:\Temp\uv";
    let protected_err =
        remover::ValidatedItem::from_whole_cache_root("wc-bad-1", "uv-cache", protected, 0)
            .unwrap_err();
    let shallow_err =
        remover::ValidatedItem::from_whole_cache_root("wc-bad-2", "uv-cache", shallow, 0)
            .unwrap_err();
    let checks = [
        protected_err.to_ascii_lowercase().contains("protect"),
        shallow_err.to_ascii_lowercase().contains("depth"),
    ];
    assert_eq!(
        checks,
        [true, true],
        "protected={protected_err} shallow={shallow_err}"
    );
}
