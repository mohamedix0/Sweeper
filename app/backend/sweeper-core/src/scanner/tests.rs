use super::*;
use crate::remover::{self, CleanOptions, ValidatedItem};
use crate::scanner::allowlist::{scan_resolved_roots, ResolvedRootsScanCtx};
use sweeper_knowledge::{KnownFolder, Risk, Scope, Tier};

/// Parameters that differ between test targets. Shared so every test target
/// is assembled by exactly one builder and cannot drift in its fixed fields.
struct TargetSpec<'a> {
    id: &'a str,
    name: &'a str,
    tier: Tier,
    risk: Risk,
    known_folder: KnownFolder,
    relative_subpath: &'a str,
    scope: Scope,
    min_age_hours: u32,
}

const WHOLE_CACHE_TEST_SPEC: TargetSpec<'static> = TargetSpec {
    id: "uv-cache",
    name: "uv Cache",
    tier: Tier::Tier2,
    risk: Risk::Caution,
    known_folder: KnownFolder::LocalAppData,
    relative_subpath: r"uv\cache",
    scope: Scope::WholeCache,
    min_age_hours: 0,
};

/// The single assembly point for test TargetDefinitions.
fn build_target(spec: &TargetSpec<'_>) -> TargetDefinition {
    TargetDefinition {
        id: spec.id.to_string(),
        name: spec.name.to_string(),
        tier: spec.tier,
        risk: spec.risk,
        known_folder: spec.known_folder,
        relative_subpath: spec.relative_subpath.to_string(),
        scope: spec.scope,
        min_age_hours: spec.min_age_hours,
        description: "test".to_string(),
        requires_elevation: false,
    }
}

fn sample_target(id: &str, rel: &str) -> TargetDefinition {
    let spec = TargetSpec {
        id,
        name: id,
        tier: Tier::Tier1,
        risk: Risk::Safe,
        known_folder: KnownFolder::LocalAppData,
        relative_subpath: rel,
        scope: Scope::Contents,
        min_age_hours: 24,
    };
    build_target(&spec)
}

fn seed_node_modules_cache(sb: &sweeper_testkit::Sandbox) -> (std::path::PathBuf, CanonicalPath) {
    let root = sb.create_dir("uv-cache").unwrap();
    std::fs::create_dir_all(root.join("node_modules")).unwrap();
    std::fs::write(root.join("node_modules").join("pkg.bin"), vec![b'x'; 4096]).unwrap();
    std::fs::write(root.join("normal.bin"), vec![b'y'; 1024]).unwrap();
    let canon = CanonicalPath::from_existing_path(&root.to_string_lossy())
        .unwrap()
        .into_canonical();
    (root, canon)
}

fn whole_cache_test_target() -> TargetDefinition {
    build_target(&WHOLE_CACHE_TEST_SPEC)
}

fn estimate_whole_cache(target: &TargetDefinition, canon: &CanonicalPath) -> u64 {
    let roots = vec![(target.id.clone(), canon.clone())];
    let mut progress =
        ScanProgressRuntime::new(1, std::sync::Arc::new(AtomicBool::new(false)), None);
    let mut ctx = ResolvedRootsScanCtx {
        target,
        roots: &roots,
        min_age: Duration::from_secs(24 * 3600),
        now: SystemTime::now(),
        progress: &mut progress,
    };
    let mut out = empty_target_result(TargetResultMeta {
        target_id: target.id.clone(),
        target_name: target.name.clone(),
        tier: target.tier,
        risk: target.risk,
    });
    scan_resolved_roots(&mut ctx, &mut out).expect("whole-cache scan");
    assert_eq!(out.items.len(), 1);
    out.items[0].size
}

fn clean_whole_cache(
    sb: &sweeper_testkit::Sandbox,
    canon: &CanonicalPath,
    estimate: u64,
) -> remover::CleanReport {
    let item =
        ValidatedItem::from_whole_cache_root("wc-nm-1", "uv-cache", canon.as_str(), estimate)
            .expect("whole-cache root must validate");
    let journal = sb.create_dir("journal").unwrap();
    remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: estimate,
            min_age: Duration::from_secs(24 * 3600),
            now: SystemTime::now(),
            journal_base_dir: Some(journal),
        },
    )
    .expect("whole-cache clean")
}

fn assert_estimate_and_clean(report: &remover::CleanReport, root: &std::path::Path, estimate: u64) {
    assert!(
        !report.aborted,
        "120% breaker must not trip: {:?}",
        report.abort_reason
    );
    assert!(!root.exists(), "whole cache root must be removed");
    assert!(
        report.bytes_freed >= estimate,
        "deleted bytes must cover the estimate: {} < {}",
        report.bytes_freed,
        estimate
    );
}

#[test]
fn whole_cache_estimate_matches_deleted_bytes_with_node_modules() {
    let sb = sweeper_testkit::Sandbox::new();
    let (root, canon) = seed_node_modules_cache(&sb);
    let estimated = estimate_whole_cache(&whole_cache_test_target(), &canon);
    assert_eq!(
        estimated,
        4096 + 1024,
        "node_modules bytes must be in the Preview estimate"
    );
    let canary =
        sweeper_testkit::CanaryTree::seed_external("whole-cache-parity").expect("seed canary");
    let report = clean_whole_cache(&sb, &canon, estimated);
    assert_estimate_and_clean(&report, &root, estimated);
    sb.assert_sentinel_present();
    canary.verify().expect("canary must remain intact");
}

#[test]
fn rejects_caller_supplied_target_shapes_via_lint() {
    let bad = vec![sample_target("bad", "Foo\\**\\Bar")];
    let err = scan_targets(bad, 24).unwrap_err().to_string();
    assert!(err.contains("**") || err.contains("glob"), "{err}");
}

#[test]
fn whole_cache_scan_emits_single_root_item_with_all_files_age_eligible() {
    let sb = sweeper_testkit::Sandbox::new();
    let root = sb.create_dir("uv-cache").unwrap();
    let nested = root.join("v1");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("a.bin"), b"aaa").unwrap();
    std::fs::write(nested.join("b.bin"), b"bbbb").unwrap();
    let canon = CanonicalPath::from_existing_path(&root.to_string_lossy())
        .unwrap()
        .into_canonical();

    let target = TargetDefinition {
        id: "uv-cache".to_string(),
        name: "uv Cache".to_string(),
        tier: Tier::Tier2,
        risk: Risk::Caution,
        known_folder: KnownFolder::LocalAppData,
        relative_subpath: r"uv\cache".to_string(),
        scope: Scope::WholeCache,
        min_age_hours: 0,
        description: "test".to_string(),
        requires_elevation: false,
    };
    let roots = vec![(target.id.clone(), canon.clone())];
    let mut progress =
        ScanProgressRuntime::new(1, std::sync::Arc::new(AtomicBool::new(false)), None);
    let mut ctx = ResolvedRootsScanCtx {
        target: &target,
        roots: &roots,
        min_age: Duration::from_secs(24 * 3600),
        now: SystemTime::now(),
        progress: &mut progress,
    };
    let mut out = empty_target_result(TargetResultMeta {
        target_id: target.id.clone(),
        target_name: target.name.clone(),
        tier: target.tier,
        risk: target.risk,
    });

    scan_resolved_roots(&mut ctx, &mut out).expect("whole-cache scan");

    assert_eq!(out.items.len(), 1, "whole cache must be a single item");
    assert_eq!(out.items[0].path, canon.as_str());
    assert!(out.items[0].age_eligible, "no per-file age filtering");
    assert_eq!(out.items[0].size, 7);
    assert_eq!(out.eligible_bytes, 7);
    assert_eq!(out.eligible_count, 1);
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
