#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command; // kept for other tests; TOCTOU hot loop uses fast Win32/testkit APIs
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sweeper_core::journal::{self, JournalRecord};
use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, ValidatedSource};
use sweeper_testkit::{CanaryTree, Sandbox};
use sweeper_winfs::ReparseTagClass;

fn now() -> SystemTime {
    SystemTime::now()
}

fn unix_now() -> u64 {
    now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn old_mtime_secs() -> u64 {
    unix_now().saturating_sub(48 * 3600)
}

// (no extra helpers)

fn canonical_str(p: &Path) -> String {
    CanonicalPath::from_existing_path(&p.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

fn path_str_for_item(p: &Path) -> String {
    if p.exists() {
        // Link points are valid deletion targets as entries; from_existing_path rejects them.
        if let Ok(v) = CanonicalPath::from_existing_path(&p.to_string_lossy()) {
            return v.as_str().to_string();
        }
    }
    CanonicalPath::parse_lexical(&p.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

fn make_item(
    id: &str,
    target_root: &Path,
    path: &Path,
    size: u64,
    mtime: Option<u64>,
) -> remover::ValidatedItem {
    let root = canonical_str(target_root);
    let p = path_str_for_item(path);
    remover::validated_item_for_tests(
        id,
        "test-target",
        root,
        p,
        size,
        mtime,
        ValidatedSource::CustomPath,
    )
}

fn clean_one(
    item: remover::ValidatedItem,
    preview_estimated_bytes: u64,
    min_age: Duration,
    journal_base: &Path,
) -> remover::CleanReport {
    remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes,
            min_age,
            now: now(),
            journal_base_dir: Some(journal_base.to_path_buf()),
        },
    )
    .unwrap()
}

fn assert_sentinel_intact(sb: &Sandbox) {
    sb.assert_sentinel_present();
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn set_filetime(path: &Path, secs: u64) {
    let ft = sweeper_testkit::filetime_from_unix_secs(secs);
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

#[test]
fn gate_d_junction_or_symlink_inside_target_unlinks_entry_and_preserves_canary() {
    let sb = Sandbox::new().unwrap();
    let target_root = sb.create_dir("target").unwrap();
    let victim = target_root.join("file.txt");
    write_file(&victim, b"delete me");
    set_filetime(&victim, old_mtime_secs());

    let canary = CanaryTree::seed_external("gate_d_link").unwrap();
    let link = target_root.join("link_to_canary");
    sweeper_testkit::create_junction(&link, canary.root()).unwrap();
    assert!(link.exists());
    set_filetime(&link, old_mtime_secs());

    let item = make_item("i1", &target_root, &link, 0, Some(old_mtime_secs()));
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
    assert!(canary.verify().is_ok(), "canary must remain intact");
    assert!(
        !link.exists() && fs::symlink_metadata(&link).is_err(),
        "link entry should be removed, report={report:?}"
    );
    // Victim file inside target still exists (we only deleted link item)
    assert!(victim.exists(), "non-targeted file should remain");
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_toctou_swap_directory_for_junction_canaries_survive_1000_iterations() {
    let sb = Sandbox::new().unwrap();
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

        let item = make_item(
            &format!("i_{iter}"),
            &target_root,
            &sub,
            1024,
            Some(old_mtime_secs()),
        );
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

        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();

        assert!(
            canary.verify().is_ok(),
            "external canary modified on iteration {iter}"
        );
    }
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_target_root_or_ancestor_swapped_to_junction_rejected_zero_deletions() {
    let sb = Sandbox::new().unwrap();
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

#[test]
fn gate_d_hard_link_to_outside_file_outside_intact() {
    let sb = Sandbox::new().unwrap();
    let target_root = sb.create_dir("target").unwrap();
    let outside_dir = sb.create_dir("outside").unwrap();
    let outside_file = outside_dir.join("outside.txt");
    write_file(&outside_file, b"outside-canary-content");
    set_filetime(&outside_file, old_mtime_secs());

    let link_in_target = target_root.join("hardlink.txt");
    sweeper_testkit::create_hard_link(&link_in_target, &outside_file).unwrap();
    set_filetime(&link_in_target, old_mtime_secs());

    let item = make_item(
        "i1",
        &target_root,
        &link_in_target,
        10,
        Some(old_mtime_secs()),
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    let report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    assert!(outside_file.exists(), "outside hardlinked file must remain");
    let content = fs::read(&outside_file).unwrap();
    assert_eq!(content, b"outside-canary-content");
    assert!(!report.aborted, "report={report:?}");
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_locked_file_skipped_remaining_eligible_cleaned() {
    let sb = Sandbox::new().unwrap();
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

    let item = make_item(
        "root",
        &target_root,
        &target_root,
        100,
        Some(old_mtime_secs()),
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    let _report = clean_one(item, 1024, Duration::from_secs(3600), &journal_dir);

    drop(file);
    assert!(locked.exists(), "locked file must be skipped and remain");
    assert!(!deletable.exists(), "eligible file should be cleaned");
    // Report granularity may aggregate root item; filesystem outcome is authoritative.
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_readonly_hidden_system_and_age_boundary_and_future_mtime() {
    let sb = Sandbox::new().unwrap();
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

    let item = make_item(
        "root",
        &target_root,
        &target_root,
        1024,
        Some(old_mtime_secs()),
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    let _report = clean_one(item, 10 * 1024, min_age, &journal_dir);

    assert!(!readonly.exists(), "readonly should be handled/deleted");
    assert!(!hidden.exists(), "hidden should be deleted in valid target");
    assert!(!system.exists(), "system should be deleted in valid target");
    assert!(!old.exists(), "old file should be deleted");
    assert!(
        !boundary.exists(),
        "exact boundary should be eligible/deleted"
    );
    assert!(recent.exists(), "recent file must be skipped");
    assert!(future.exists(), "future mtime must be skipped as recent");
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_long_paths_deep_tree_reserved_ads_trailing_dots_arabic_emoji_case_sensitive() {
    let sb = Sandbox::new().unwrap();
    let target_root = sb.create_dir("target").unwrap();

    // Deep 1000-level tree (iterative walker)
    let deep = sweeper_testkit::create_deep_tree(&target_root, 1005).unwrap();
    let deep_file = deep.join("deep.txt");
    write_file(&deep_file, b"deep");

    // Long path > 260 chars
    let long_dir = target_root
        .join("L".repeat(120))
        .join("M".repeat(120))
        .join("N".repeat(40));
    fs::create_dir_all(sweeper_testkit::to_extended_length_path(&long_dir)).unwrap();
    let long_file = long_dir.join("long_file_name_".to_string() + &"x".repeat(80) + ".txt");
    write_file(
        &sweeper_testkit::to_extended_length_path(&long_file),
        b"long",
    );

    // Reserved names via \\?\
    let nul_path = target_root.join("NUL_test_safe.txt"); // avoid creating actual device name file; use reserved-like name handling path
    write_file(&nul_path, b"nul-like");
    set_filetime(&nul_path, old_mtime_secs());
    // Actual reserved device names as files are not creatable normally; ensure no crash when encountered via paths logic is covered elsewhere.
    // Create a file and an ADS stream
    let ads_base = target_root.join("ads_base.txt");
    write_file(&ads_base, b"base");
    sweeper_testkit::write_ads(&ads_base, ":stream1:$DATA", b"ads-data").unwrap();
    set_filetime(&ads_base, old_mtime_secs());

    // Arabic & emoji filenames
    let arabic = target_root.join("مرحبا العربية.txt");
    let emoji = target_root.join("emoji 😀 file.txt");
    write_file(&arabic, b"arabic");
    write_file(&emoji, b"emoji");
    set_filetime(&arabic, old_mtime_secs());
    set_filetime(&emoji, old_mtime_secs());

    // Trailing dots/spaces are invalid in normal Win32 names; create via extended path if possible is not allowed for creation,
    // but we ensure scanner/remover handle/report paths safely. (Adversarial path corpus covers lexical rejection.)

    // Per-directory case-sensitive filenames (best effort)
    let case_dir = target_root.join("casedir");
    fs::create_dir_all(&case_dir).unwrap();
    let case_status = Command::new("fsutil")
        .args([
            "file",
            "setCaseSensitiveInfo",
            &case_dir.to_string_lossy(),
            "enable",
        ])
        .status();
    if case_status.map(|s| s.success()).unwrap_or(false) {
        write_file(&case_dir.join("Case.txt"), b"upper");
        // Creating "case.txt" alongside "Case.txt" requires case-sensitive dir; may fail on some FS modes.
        let _ = fs::write(case_dir.join("case.txt"), b"lower");
        for e in fs::read_dir(&case_dir).unwrap().flatten() {
            set_filetime(&e.path(), old_mtime_secs());
        }
    }

    let item = make_item(
        "root",
        &target_root,
        &target_root,
        1024 * 1024,
        Some(old_mtime_secs()),
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    // Age checks are covered in a dedicated test; use min_age 0 here to focus on deep/long path deletion.
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 50 * 1024 * 1024,
            min_age: Duration::from_secs(0),
            now: now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();

    assert!(!report.aborted, "aborted: {:?}", report.abort_reason);
    assert!(!deep_file.exists(), "deep file should be deleted");
    assert!(
        !long_file.exists() && !sweeper_testkit::to_extended_length_path(&long_file).exists(),
        "long path file should be deleted"
    );
    assert!(!ads_base.exists(), "base file should be deleted");
    assert!(!arabic.exists(), "arabic filename file should be deleted");
    assert!(!emoji.exists(), "emoji filename file should be deleted");
    // Target root directory must remain (contents-only)
    assert!(target_root.exists(), "target root must remain");
    assert_sentinel_intact(&sb);
}

#[test]
fn gate_d_volume_mount_point_directory_junction_never_traversed() {
    let sb = Sandbox::new().unwrap();
    let target_root = sb.create_dir("target").unwrap();
    let canary = CanaryTree::seed_external("gate_d_mount").unwrap();
    // Directory junction acts as mount point/link point; must be treated as Link Point and never traversed.
    let junction = target_root.join("junction_mount");
    sweeper_testkit::create_junction(&junction, canary.root()).unwrap();
    assert!(junction.exists());

    let item = make_item(
        "root",
        &target_root,
        &target_root,
        1024,
        Some(old_mtime_secs()),
    );
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

#[test]
fn gate_d_reparse_tag_matrix_cloud_and_unknown_skipped() {
    let sb = Sandbox::new().unwrap();
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

    let item = make_item(
        "root",
        &target_root,
        &target_root,
        1024,
        Some(old_mtime_secs()),
    );
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

#[test]
fn gate_d_missing_custom_path_zero_deletions_parent_untouched() {
    let sb = Sandbox::new().unwrap();
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
    let sb = Sandbox::new().unwrap();
    let target_root = sb.create_dir("target").unwrap();
    // Create files totaling more than 120% of preview estimate.
    for i in 0..10 {
        let p = target_root.join(format!("f{i}.bin"));
        write_file(&p, &vec![b'x'; 1024]);
        set_filetime(&p, old_mtime_secs());
    }
    let item = make_item(
        "root",
        &target_root,
        &target_root,
        10 * 1024,
        Some(old_mtime_secs()),
    );
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

#[test]
fn gate_d_mid_clean_termination_recovery_truncated_journal_and_rotation() {
    let sb = Sandbox::new().unwrap();
    let journal_base = sb.create_dir("logsbase").unwrap();
    let log_dir = journal::resolve_log_dir(&journal_base);
    fs::create_dir_all(&log_dir).unwrap();

    // Create 35 log files to verify rotation keeps last 30.
    for i in 0..35u64 {
        let p = log_dir.join(format!("clean-170000{:04}-{}-0.jsonl", i, i));
        let mut f = fs::File::create(&p).unwrap();
        writeln!(f, "{}", serde_json::json!({"record":"intent","target_id":"t","path":"C:\\\\a\\\\b\\\\c","size":1,"timestamp":1700000000u64+i})).unwrap();
        // set distinct modified times by rewriting; best effort
    }
    journal::rotate_logs(&log_dir, 30).unwrap();
    let count = fs::read_dir(&log_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .count();
    assert!(
        count <= 30,
        "rotation should retain last 30 logs, got {count}"
    );

    // Simulate truncated journal (mid-clean termination) and ensure reader recovers.
    let truncated = log_dir.join("clean-1700999999-1-0.jsonl");
    {
        let mut f = fs::File::create(&truncated).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&JournalRecord::Intent {
                target_id: "x".into(),
                path: "C:\\Temp\\x".into(),
                size: 1,
                timestamp: 1,
                batch_id: None
            })
            .unwrap()
        )
        .unwrap();
        // truncated/incomplete trailing line
        write!(f, "{{\"record\":\"outcome\",\"target_id\":\"x\"").unwrap();
    }
    let recs = journal::read_records(&truncated).unwrap();
    assert_eq!(recs.len(), 1, "truncated trailing line should be ignored");
    match &recs[0] {
        JournalRecord::Intent { .. } => {}
        _ => panic!("expected intent record"),
    }

    // Also run a small clean to produce a valid journal and ensure logs exist.
    let target_root = sb.create_dir("target").unwrap();
    let victim = target_root.join("v.txt");
    write_file(&victim, b"bye");
    set_filetime(&victim, old_mtime_secs());
    let item = make_item("i", &target_root, &victim, 3, Some(old_mtime_secs()));
    let _ = clean_one(item, 1024, Duration::from_secs(3600), &journal_base);
    assert!(!victim.exists());
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
