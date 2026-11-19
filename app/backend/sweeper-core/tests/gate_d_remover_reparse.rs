#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::path::Path;
use std::time::Duration;

use sweeper_testkit::Sandbox;
use sweeper_winfs::ReparseTagClass;

mod common;
use common::*;

#[test]
fn gate_d_reparse_tag_matrix_cloud_and_unknown_skipped() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let cloud_file = target_root.join("cloud.txt");
    let unknown_file = target_root.join("unknown_reparse.txt");
    write_file(&cloud_file, b"cloud");
    write_file(&unknown_file, b"unknown");
    set_filetime(&cloud_file, old_mtime_secs());
    set_filetime(&unknown_file, old_mtime_secs());

    // Deterministic hook for tag matrix (mocked cloud placeholder tag / unknown reparse tag).
    let cloud_canon = if cloud_file.exists() {
        canonical_str(&cloud_file)
    } else {
        cloud_file.to_string_lossy().to_string()
    };
    let unknown_canon = if unknown_file.exists() {
        canonical_str(&unknown_file)
    } else {
        unknown_file.to_string_lossy().to_string()
    };
    sweeper_winfs::set_test_reparse_override(
        Path::new(&cloud_canon),
        Some(ReparseTagClass::CloudPlaceholder),
    );
    sweeper_winfs::set_test_reparse_override(
        Path::new(&unknown_canon),
        Some(ReparseTagClass::OtherReparse(0xE000_0001)),
    );
    sweeper_winfs::set_test_reparse_override(&cloud_file, Some(ReparseTagClass::CloudPlaceholder));
    sweeper_winfs::set_test_reparse_override(
        &unknown_file,
        Some(ReparseTagClass::OtherReparse(0xE000_0001)),
    );

    let item = make_item("root", &target_root, &target_root, 1024);
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    assert!(
        cloud_file.exists(),
        "cloud placeholder must be skipped, report={report:?}"
    );
    assert!(
        unknown_file.exists(),
        "unknown reparse tag must be skipped, report={report:?}"
    );

    sweeper_winfs::set_test_reparse_override(&cloud_file, None);
    sweeper_winfs::set_test_reparse_override(&unknown_file, None);
    assert_sentinel_intact(&sb);

    // Verify classify_reparse_tag matrix for known cloud tag variants.
    assert!(matches!(
        sweeper_winfs::classify_reparse_tag(0x9000_001A),
        ReparseTagClass::CloudPlaceholder
    ));
    assert!(matches!(
        sweeper_winfs::classify_reparse_tag(0x9000_301A),
        ReparseTagClass::CloudPlaceholder
    ));
    assert!(matches!(
        sweeper_winfs::classify_reparse_tag(0xE000_0001),
        ReparseTagClass::OtherReparse(_)
    ));
}
