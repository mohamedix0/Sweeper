#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::path::Path;
use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::{CanaryTree, Sandbox};

mod common;
use common::*;

fn assert_link_clean_canary_intact(canary: &CanaryTree, link: &Path, victim: &Path) {
    assert!(canary.verify().is_ok(), "canary must remain intact");
    assert!(
        !link.exists() && fs::symlink_metadata(link).is_err(),
        "link entry should be removed"
    );
    assert!(victim.exists(), "non-targeted file should remain");
}

#[test]
fn gate_d_junction_or_symlink_inside_target_unlinks_entry_and_preserves_canary() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let victim = target_root.join("file.txt");
    write_file(&victim, b"delete me");
    set_filetime(&victim, old_mtime_secs());

    let canary = CanaryTree::seed_external("gate_d_link").unwrap();
    let link = target_root.join("link_to_canary");
    sweeper_testkit::create_junction(&link, canary.root()).unwrap();
    assert!(link.exists());
    set_filetime(&link, old_mtime_secs());

    let item = make_item("i1", &target_root, &link, 0);
    let journal_dir = sb.create_dir("journal").unwrap();
    // Age not the focus of this safety test; link point timestamps can be hard to set portably.
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(0),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    assert!(!report.aborted, "aborted: {:?}", report.abort_reason);
    // Victim file inside target still exists (we only deleted link item).
    assert_link_clean_canary_intact(&canary, &link, &victim);
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_hard_link_to_outside_file_outside_intact() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let outside_dir = sb.create_dir("outside").unwrap();
    let outside_file = outside_dir.join("outside.txt");
    write_file(&outside_file, b"outside-canary-content");
    set_filetime(&outside_file, old_mtime_secs());

    let link_in_target = target_root.join("hardlink.txt");
    sweeper_testkit::create_hard_link(&link_in_target, &outside_file).unwrap();
    set_filetime(&link_in_target, old_mtime_secs());

    let item = make_item("i1", &target_root, &link_in_target, 10);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    assert!(outside_file.exists(), "outside hardlinked file must remain");
    let content = fs::read(&outside_file).unwrap();
    assert_eq!(content, b"outside-canary-content");
    assert!(!report.aborted, "report={report:?}");
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_volume_mount_point_directory_junction_never_traversed() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let canary = CanaryTree::seed_external("gate_d_mount").unwrap();
    // Directory junction acts as mount point/link point; must be treated as Link Point and never traversed.
    let junction = target_root.join("junction_mount");
    sweeper_testkit::create_junction(&junction, canary.root()).unwrap();
    assert!(junction.exists());

    let item = make_item("root", &target_root, &target_root, 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024 * 1024, Duration::from_secs(3600), &journal_dir);

    assert!(
        canary.verify().is_ok(),
        "junction target contents must not be traversed/deleted"
    );
    // Contents must never be traversed; link entry may be unlinked.
    assert!(
        !report.aborted
            || report
                .abort_reason
                .as_deref()
                .unwrap_or("")
                .contains("120%"),
        "unexpected abort: {report:?}"
    );
    assert_sentinel_intact(&sb);
}
