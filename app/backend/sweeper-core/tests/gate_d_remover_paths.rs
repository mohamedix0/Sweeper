#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::process::Command;
use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

fn seed_deep_and_long(target_root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let deep = sweeper_testkit::create_deep_tree(target_root, 1005).unwrap();
    let deep_file = deep.join("deep.txt");
    write_file(&deep_file, b"deep");
    let long_dir = target_root
        .join("L".repeat(120))
        .join("M".repeat(120))
        .join("N".repeat(40));
    fs::create_dir_all(sweeper_testkit::to_extended_length_path(&long_dir)).unwrap();
    let long_file = long_dir.join(format!("long_file_name_{}.txt", "x".repeat(80)));
    write_file(
        &sweeper_testkit::to_extended_length_path(&long_file),
        b"long",
    );
    (deep_file, long_file)
}

fn seed_special_names(
    target_root: &std::path::Path,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let nul_path = target_root.join("NUL_test_safe.txt");
    write_file(&nul_path, b"nul-like");
    set_filetime(&nul_path, old_mtime_secs());
    let ads_base = target_root.join("ads_base.txt");
    write_file(&ads_base, b"base");
    sweeper_testkit::write_ads(&ads_base, ":stream1:$DATA", b"ads-data").unwrap();
    set_filetime(&ads_base, old_mtime_secs());
    let arabic = target_root.join("\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} \u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064a}\u{0629}.txt");
    let emoji = target_root.join("emoji \u{1F600} file.txt");
    write_file(&arabic, b"arabic");
    write_file(&emoji, b"emoji");
    set_filetime(&arabic, old_mtime_secs());
    set_filetime(&emoji, old_mtime_secs());
    (ads_base, arabic, emoji)
}

fn try_enable_case_sensitive_dir(target_root: &std::path::Path) {
    let case_dir = target_root.join("casedir");
    fs::create_dir_all(&case_dir).unwrap();
    let status = Command::new("fsutil")
        .args([
            "file",
            "setCaseSensitiveInfo",
            &case_dir.to_string_lossy(),
            "enable",
        ])
        .status();
    if !status.map(|s| s.success()).unwrap_or(false) {
        return;
    }
    write_file(&case_dir.join("Case.txt"), b"upper");
    let _ = fs::write(case_dir.join("case.txt"), b"lower");
    for e in fs::read_dir(&case_dir).unwrap().flatten() {
        set_filetime(&e.path(), old_mtime_secs());
    }
}

fn run_long_path_clean(
    target_root: &std::path::Path,
    journal_dir: std::path::PathBuf,
) -> sweeper_core::remover::CleanReport {
    let item = make_item("root", target_root, target_root, 1024 * 1024);
    remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 50 * 1024 * 1024,
            min_age: Duration::from_secs(0),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap()
}

#[test]
fn gate_d_long_paths_deep_tree_reserved_ads_trailing_dots_arabic_emoji_case_sensitive() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let (deep_file, long_file) = seed_deep_and_long(&target_root);
    let (ads_base, arabic, emoji) = seed_special_names(&target_root);
    try_enable_case_sensitive_dir(&target_root);
    let report = run_long_path_clean(&target_root, sb.create_dir("journal").unwrap());
    assert!(!report.aborted, "aborted: {:?}", report.abort_reason);
    assert!(!deep_file.exists(), "deep file should be deleted");
    let long_gone =
        !long_file.exists() && !sweeper_testkit::to_extended_length_path(&long_file).exists();
    assert!(long_gone, "long path file should be deleted");
    assert!(!ads_base.exists(), "base file should be deleted");
    assert!(!arabic.exists(), "arabic filename file should be deleted");
    let emoji_gone = !emoji.exists();
    assert!(emoji_gone, "emoji filename file should be deleted");
    assert!(target_root.exists(), "target root must remain");
    assert_sentinel_intact(&sb);
}

/// D1 follow-up: on a case-sensitive directory, a junction to a sibling whose
/// name differs only by case is a distinct entry. The folded path comparison
/// alone cannot see the swap, so Clean must abort and leave the sibling intact.
#[test]
fn gate_d_case_distinct_sibling_swap_is_aborted_and_canary_survives() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();

    try_enable_case_sensitive_dir(&target_root);
    let case_dir = target_root.join("casedir");
    let names: Vec<String> = fs::read_dir(&case_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    if !(names.iter().any(|name| name == "Case.txt") && names.iter().any(|name| name == "case.txt"))
    {
        eprintln!("skipping: case-sensitive directory unavailable (requires elevation)");
        return;
    }

    let sub = case_dir.join("sub");
    let sibling = case_dir.join("SUB");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(&sibling).unwrap();
    let victim = sub.join("victim.txt");
    let canary = sibling.join("victim.txt");
    write_file(&victim, b"confirmed selection");
    write_file(&canary, b"never selected");
    set_filetime(&victim, old_mtime_secs());
    set_filetime(&sub, old_mtime_secs());
    set_filetime(&canary, old_mtime_secs());
    let item = make_item("case-swap", &target_root, &victim, 1024);

    // Swap the confirmed directory for a junction to the case-distinct sibling.
    fs::remove_file(&victim).unwrap();
    fs::remove_dir(&sub).unwrap();
    sweeper_testkit::create_junction(&sub, &sibling).unwrap();

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

    assert!(
        report.aborted,
        "case-variant redirect must abort: {report:?}"
    );
    assert_eq!(
        report.bytes_freed, 0,
        "no unselected bytes freed: {report:?}"
    );
    assert!(
        report.items.iter().all(|row| row.bytes_deleted == 0),
        "no outcome may record a deletion: {report:?}"
    );
    assert!(canary.exists(), "case-distinct sibling canary must survive");
    assert_sentinel_intact(&sb);
}
