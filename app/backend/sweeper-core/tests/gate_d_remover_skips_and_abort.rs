#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::time::Duration;

use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, ValidatedSource};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

#[test]
fn gate_d_locked_file_skipped_remaining_eligible_cleaned() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let locked = target_root.join("locked.txt");
    let deletable = target_root.join("deletable.txt");
    write_file(&locked, b"locked");
    write_file(&deletable, b"deletable");
    set_filetime(&locked, old_mtime_secs());
    set_filetime(&deletable, old_mtime_secs());

    // Open without FILE_SHARE_DELETE to lock deletion.
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0x1 | 0x2)
        .open(&locked)
        .unwrap();

    let item = make_item("root", &target_root, &target_root, 100);
    let journal_dir = sb.create_dir("journal").unwrap();
    let _report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    drop(file);
    assert!(locked.exists(), "locked file must be skipped and remain");
    assert!(!deletable.exists(), "eligible file should be cleaned");
    // Report granularity may aggregate root item; filesystem outcome is authoritative.
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_missing_custom_path_zero_deletions_parent_untouched() {
    let sb = Sandbox::new();
    let parent = sb.create_dir("parent").unwrap();
    let parent_file = parent.join("keep.txt");
    write_file(&parent_file, b"keep");
    set_filetime(&parent_file, old_mtime_secs());
    let missing_root = parent.join("missing_custom_path_root");
    assert!(!missing_root.exists());

    let root_str = CanonicalPath::parse_lexical(&missing_root.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string();
    let item = remover::validated_item_for_tests(
        "i1",
        "custom",
        root_str.clone(),
        root_str,
        123,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    assert_eq!(report.bytes_freed, 0, "zero deletions expected: {report:?}");
    assert!(
        parent.exists() && parent_file.exists(),
        "parent must be untouched"
    );
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_120_percent_circuit_breaker_aborts_cleanly() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    // Create files totaling more than 120% of preview estimate.
    for i in 0..10 {
        let p = target_root.join(format!("f{i}.bin"));
        write_file(&p, &vec![b'x'; 1024]);
        set_filetime(&p, old_mtime_secs());
    }
    let item = make_item("root", &target_root, &target_root, 10 * 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1000, // 120% = 1200 bytes; actual much larger
            min_age: Duration::from_secs(3600),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    assert!(
        report.aborted,
        "expected circuit breaker abort, report={report:?}"
    );
    assert!(
        report
            .abort_reason
            .as_deref()
            .unwrap_or("")
            .contains("120%"),
        "abort reason should mention 120%: {:?}",
        report.abort_reason
    );
    // Some files may remain because we aborted; key is clean abort without escaping sandbox.
    assert_sentinel_intact(&sb);
}

// Ensure test uses std::os::windows::fs::OpenOptionsExt for share_mode
trait OpenOptionsShareExt {
    fn share_mode(&mut self, val: u32) -> &mut Self;
}
impl OpenOptionsShareExt for fs::OpenOptions {
    fn share_mode(&mut self, val: u32) -> &mut Self {
        std::os::windows::fs::OpenOptionsExt::share_mode(self, val)
    }
}
