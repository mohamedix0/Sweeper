use super::*;
use sweeper_knowledge::{KnownFolder, Risk, Scope, Tier};

fn sample_target(id: &str, rel: &str) -> TargetDefinition {
    TargetDefinition {
        id: id.to_string(),
        name: id.to_string(),
        tier: Tier::Tier1,
        risk: Risk::Safe,
        known_folder: KnownFolder::LocalAppData,
        relative_subpath: rel.to_string(),
        scope: Scope::Contents,
        min_age_hours: 24,
        description: "test".to_string(),
        requires_elevation: false,
    }
}

#[test]
fn rejects_caller_supplied_target_shapes_via_lint() {
    let bad = vec![sample_target("bad", "Foo\\**\\Bar")];
    let err = scan_targets(bad, 24).unwrap_err().to_string();
    assert!(err.contains("**") || err.contains("glob"), "{err}");
}

fn assert_redirected_temp_guard_disabled(candidate: &str, expected_reason_fragment: &str) {
    let local = sweeper_winfs::resolve_known_folder(KnownFolderId::LocalAppData)
        .expect("resolve local appdata");
    let user_temp = CanonicalPath::from_existing_path(&local.join("Temp").to_string_lossy())
        .expect("user temp")
        .into_canonical();
    let guard = evaluate_redirected_temp_candidate(Some(candidate), &user_temp);
    match guard {
        TempGuard::Disabled(reason) => {
            assert!(reason.contains(expected_reason_fragment), "{reason}");
        }
        TempGuard::Use(p) => panic!("expected disabled, got {p}"),
    }
}

#[test]
fn redirected_temp_guard_depth_lt_3_drive_root_disabled() {
    assert_redirected_temp_guard_disabled(r"C:\", "depth < 3");
}

#[test]
fn redirected_temp_guard_identical_to_user_temp_disabled() {
    let local = sweeper_winfs::resolve_known_folder(KnownFolderId::LocalAppData)
        .expect("resolve local appdata");
    let user_temp = CanonicalPath::from_existing_path(&local.join("Temp").to_string_lossy())
        .expect("user temp")
        .into_canonical();
    assert_redirected_temp_guard_disabled(user_temp.as_str(), "identical");
}

#[test]
fn redirected_temp_guard_profile_root_disabled_as_protected_container() {
    let local = sweeper_winfs::resolve_known_folder(KnownFolderId::LocalAppData)
        .expect("resolve local appdata");
    let user_temp = CanonicalPath::from_existing_path(&local.join("Temp").to_string_lossy())
        .expect("user temp")
        .into_canonical();
    let profile =
        sweeper_winfs::resolve_known_folder(KnownFolderId::UserProfile).expect("resolve profile");
    let guard = evaluate_redirected_temp_candidate(Some(&profile.to_string_lossy()), &user_temp);
    match guard {
        TempGuard::Disabled(reason) => {
            assert!(
                reason.contains("protected") || reason.contains("depth < 3"),
                "{reason}"
            )
        }
        TempGuard::Use(p) => panic!("expected disabled, got {p}"),
    }
}

#[test]
fn progress_callback_invoked_for_target_lifecycle() {
    let target = sample_target("missing-target-progress", "SweeperNonExistentProgress\\Sub");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<ScanProgress>::new()));
    let seen_cb = seen.clone();
    let cb: ScanProgressCallback = std::sync::Arc::new(move |p| {
        seen_cb.lock().unwrap().push(p);
    });
    let report = scan_targets_with_redirected_temp_and_elevation_overrides_with_progress(
        vec![target],
        AllowlistedScanControl {
            min_age_hours: 24,
            redirected_temp_candidate: None,
            elevation_target_root_overrides: &BTreeMap::new(),
            cancel: std::sync::Arc::new(AtomicBool::new(false)),
            progress_cb: Some(cb),
        },
    )
    .expect("scan should succeed");
    assert_eq!(report.targets.len(), 1);
    let events = seen.lock().unwrap();
    assert!(!events.is_empty(), "expected progress callbacks");
    assert_eq!(events[0].total_targets, 1);
    assert_eq!(
        events[0].current_target_id.as_deref(),
        Some("missing-target-progress")
    );
    let last = events.last().unwrap();
    assert_eq!(last.completed_targets, 1);
    assert_eq!(last.total_targets, 1);
}

#[test]
fn cooperative_cancel_returns_cancelled_error() {
    let targets = vec![
        sample_target("missing-cancel-a", "SweeperNonExistentCancel\\A"),
        sample_target("missing-cancel-b", "SweeperNonExistentCancel\\B"),
    ];
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let cancel_cb = cancel.clone();
    let cb: ScanProgressCallback = std::sync::Arc::new(move |_p| {
        cancel_cb.store(true, Ordering::Relaxed);
    });
    let err = scan_targets_with_redirected_temp_and_elevation_overrides_with_progress(
        targets,
        AllowlistedScanControl {
            min_age_hours: 24,
            redirected_temp_candidate: None,
            elevation_target_root_overrides: &BTreeMap::new(),
            cancel,
            progress_cb: Some(cb),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ScanError::Cancelled));
}

#[test]
fn settings_scan_cancel_interrupts_custom_path_phase() {
    let settings = crate::settings::SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: false,
        custom_paths: vec![r"C:\SweeperNonExistentCustomPathCancel\Nested".to_string()],
        ..crate::settings::SweeperSettings::default()
    };

    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let cancel_cb = cancel.clone();
    let cb: ScanProgressCallback = std::sync::Arc::new(move |p| {
        if p.current_target_id.as_deref() == Some("custom-path-1") {
            cancel_cb.store(true, Ordering::Relaxed);
        }
    });
    let err = scan_with_settings_options_and_elevation_with_progress(
        &settings,
        ScanSettingsOptions::default(),
        crate::ScanElevationContext::default(),
        ScanRunControl {
            cancel,
            progress_cb: Some(cb),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ScanError::Cancelled));
}

#[test]
fn settings_scan_cancel_interrupts_recycle_bin_phase() {
    let settings = crate::settings::SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        ..crate::settings::SweeperSettings::default()
    };

    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let cancel_cb = cancel.clone();
    let cb: ScanProgressCallback = std::sync::Arc::new(move |p| {
        if p.current_target_id.as_deref() == Some("recycle-bin") {
            cancel_cb.store(true, Ordering::Relaxed);
        }
    });
    let opts = ScanSettingsOptions {
        recycle_bin_roots: vec![r"C:\SweeperNonExistentRecycleBinCancel\Root".to_string()],
        ..ScanSettingsOptions::default()
    };
    let err = scan_with_settings_options_and_elevation_with_progress(
        &settings,
        opts,
        crate::ScanElevationContext::default(),
        ScanRunControl {
            cancel,
            progress_cb: Some(cb),
        },
    )
    .unwrap_err();
    assert!(matches!(err, ScanError::Cancelled));
}

#[test]
fn summarize_directory_for_scan_honors_cancellation() {
    use std::fs;
    let td = tempfile::tempdir().expect("tempdir");
    let root = td.path().join("stale-target");
    fs::create_dir_all(root.join("nested")).expect("nested dir");
    for i in 0..200u32 {
        fs::write(root.join(format!("f{i}.bin")), vec![b'x'; 16]).expect("write file");
    }
    for i in 0..50u32 {
        fs::write(root.join("nested").join(format!("n{i}.bin")), vec![b'y'; 8])
            .expect("write nested file");
    }
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let cancel_cb = cancel.clone();
    let cb: ScanProgressCallback = std::sync::Arc::new(move |_p| {
        cancel_cb.store(true, Ordering::Relaxed);
    });
    let mut progress = super::ScanProgressRuntime::new(1, cancel, Some(cb));
    let err = super::custom_tier3::summarize_directory_for_scan(
        &root,
        &mut progress,
        Some("stale-rust-target"),
        Some("Stale Rust target/"),
    )
    .unwrap_err();
    assert!(matches!(err, ScanError::Cancelled));
}
