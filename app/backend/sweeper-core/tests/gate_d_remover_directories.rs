#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

#[test]
fn gate_d_tier3_project_directory_removed_after_ntfs_mtime_bump_on_emptied_dirs() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("proj").unwrap();
    let stale_dir = target_root.join("target");
    let nested = stale_dir.join("nested").join("deep");
    fs::create_dir_all(&nested).unwrap();
    let f1 = stale_dir.join("a.bin");
    let f2 = nested.join("b.bin");
    write_file(&f1, b"aaa");
    write_file(&f2, b"bbbb");
    for p in [
        &f1,
        &f2,
        &nested,
        &nested.parent().unwrap().to_path_buf(),
        &stale_dir,
    ] {
        set_filetime(p, old_mtime_secs());
    }

    let stale_canon = canonical_str(&stale_dir);
    let item = remover::ValidatedItem::from_tier3_project_directory(
        "stale-target",
        "stale-rust-target",
        stale_canon.clone(),
        7,
        Some(old_mtime_secs()),
    )
    .unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();
    let clean_now = now();
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(3600),
            now: clean_now,
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    assert!(!report.aborted, "aborted: {:?}", report.abort_reason);
    assert!(
        !stale_dir.exists(),
        "tier 3 project directory root should be removed after contents cleaned: {:?}",
        report.items
    );
    assert_sentinel_intact(&sb);
}
