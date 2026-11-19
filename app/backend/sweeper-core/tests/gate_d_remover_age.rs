#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::path::Path;
use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

fn assert_old_and_boundary_deleted(old_deleted: bool, boundary: &Path) {
    assert!(old_deleted, "old file should be deleted");
    assert!(
        !boundary.exists(),
        "exact boundary should be eligible/deleted"
    );
}

fn assert_recent_and_future_skipped(recent: &Path, future: &Path) {
    assert!(recent.exists(), "recent file must be skipped");
    assert!(future.exists(), "future mtime must be skipped as recent");
}

#[test]
fn gate_d_readonly_hidden_system_and_age_boundary_and_future_mtime() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();

    let readonly = target_root.join("readonly.txt");
    let hidden = target_root.join("hidden.txt");
    let system = target_root.join("system.txt");
    let recent = target_root.join("recent.txt");
    let boundary = target_root.join("boundary.txt");
    let future = target_root.join("future.txt");
    let old = target_root.join("old.txt");
    for p in [
        &readonly, &hidden, &system, &recent, &boundary, &future, &old,
    ] {
        write_file(p, b"x");
    }

    let min_age = Duration::from_secs(3600);
    let t = unix_now();
    set_filetime(&readonly, t - 7200);
    set_filetime(&hidden, t - 7200);
    set_filetime(&system, t - 7200);
    set_filetime(&old, t - 7200);
    set_filetime(&recent, t - 60); // newer than min age -> skipped
    set_filetime(&boundary, t - 3600); // exact boundary: eligible when age >= min_age
    set_filetime(&future, t + 3600); // future mtime -> skipped

    sweeper_testkit::set_file_attributes(&readonly, 0x1).unwrap();
    sweeper_testkit::set_file_attributes(&hidden, 0x2).unwrap();
    sweeper_testkit::set_file_attributes(&system, 0x4).unwrap();

    let item = make_item("root", &target_root, &target_root, 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let _report = clean_one(item, 10 * 1024, min_age, &journal_dir);

    assert!(!readonly.exists(), "readonly should be handled/deleted");
    assert!(!hidden.exists(), "hidden should be deleted in valid target");
    assert!(!system.exists(), "system should be deleted in valid target");
    let old_deleted = !old.exists();
    assert_old_and_boundary_deleted(old_deleted, &boundary);
    assert_recent_and_future_skipped(&recent, &future);
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_age_checks_still_skip_unemptied_or_non_emptied_entries() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let min_age = Duration::from_secs(3600);
    let now_ts = unix_now();

    // Already-empty directory with future mtime: must be skipped.
    let empty_future = target_root.join("empty_future");
    fs::create_dir_all(&empty_future).unwrap();
    set_filetime(&empty_future, now_ts + 7200);

    // Already-empty directory newer than min_age: must be skipped.
    let empty_recent = target_root.join("empty_recent");
    fs::create_dir_all(&empty_recent).unwrap();
    set_filetime(&empty_recent, now_ts - 60);

    // Files with future/recent mtime: must be skipped.
    let future_file = target_root.join("future.txt");
    let recent_file = target_root.join("recent.txt");
    write_file(&future_file, b"f");
    write_file(&recent_file, b"r");
    set_filetime(&future_file, now_ts + 7200);
    set_filetime(&recent_file, now_ts - 60);

    // Directory containing an unemptied recent/future file: file must remain, so dir not emptied.
    let has_recent = target_root.join("has_recent");
    fs::create_dir_all(&has_recent).unwrap();
    let recent_child = has_recent.join("recent_child.txt");
    write_file(&recent_child, b"x");
    set_filetime(&recent_child, now_ts - 30);
    set_filetime(&has_recent, old_mtime_secs());

    let journal_dir = sb.create_dir("journal").unwrap();
    for (id, path) in [
        ("empty_future", &empty_future),
        ("empty_recent", &empty_recent),
        ("future_file", &future_file),
        ("recent_file", &recent_file),
        ("has_recent", &has_recent),
    ] {
        let item = make_item(id, &target_root, path, 1);
        let report = remover::execute_clean(
            vec![item],
            CleanOptions {
                preview_estimated_bytes: 1024 * 1024,
                min_age,
                now: now(),
                journal_base_dir: Some(journal_dir.clone()),
            },
        )
        .unwrap();
        assert!(
            !report.aborted,
            "aborted for {id}: {:?}",
            report.abort_reason
        );
    }

    assert!(
        empty_future.exists(),
        "empty future-mtime dir must be skipped"
    );
    assert!(empty_recent.exists(), "empty recent dir must be skipped");
    assert!(future_file.exists(), "future-mtime file must be skipped");
    let recent_file_kept = recent_file.exists();
    assert!(recent_file_kept, "recent file must be skipped");
    assert!(
        has_recent.exists() && recent_child.exists(),
        "directory containing unemptied recent file must remain"
    );
    assert_sentinel_intact(&sb);
}
