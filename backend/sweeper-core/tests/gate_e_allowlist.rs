use std::fs;
use std::process::Command;
use std::time::{Duration, SystemTime};

use sweeper_core::paths::CanonicalPath;
use sweeper_core::policy;
use sweeper_core::scanner;
use sweeper_knowledge::{KnownFolder, Risk, Scope, TargetDefinition, Tier};
use sweeper_testkit::CanaryTree;

fn set_file_mtime(path: &std::path::Path, t: SystemTime) {
    let dt = humantime_like_rfc3339(t);
    let ps = format!(
        "(Get-Item -LiteralPath '{}').LastWriteTime = [DateTime]::Parse('{}')",
        path.to_string_lossy().replace('\'', "''"),
        dt
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .status()
        .expect("run powershell to set mtime");
    assert!(
        status.success(),
        "failed to set mtime for {}",
        path.display()
    );
}

fn humantime_like_rfc3339(t: SystemTime) -> String {
    // Convert to local datetime string PowerShell can parse reliably via .NET.
    // Use RFC3339-ish UTC then let Parse handle; easier: use ToString('o') equivalent by formatting ourselves.
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // Format as yyyy-MM-ddTHH:mm:ssZ in UTC via PowerShell-independent calculation.
    // Simple civil-from-days algorithm.
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = sod / 3600;
    let mm = (sod % 3600) / 60;
    let ss = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

fn make_target(id: &str, kf: KnownFolder, rel: &str, min_age_hours: u32) -> TargetDefinition {
    TargetDefinition {
        id: id.to_string(),
        name: id.to_string(),
        tier: Tier::Tier1,
        risk: Risk::Safe,
        known_folder: kf,
        relative_subpath: rel.to_string(),
        scope: Scope::Contents,
        min_age_hours,
        description: "test target".to_string(),
        requires_elevation: false,
    }
}

fn cleanup_dir_best_effort(path: &std::path::Path) {
    let _ = Command::new("cmd")
        .args(["/C", "rmdir", "/S", "/Q", path.to_string_lossy().as_ref()])
        .status();
}

fn cleanup_file_best_effort(path: &std::path::Path) {
    let _ = Command::new("cmd")
        .args(["/C", "del", "/F", "/Q", path.to_string_lossy().as_ref()])
        .status();
}

#[test]
fn tier1_tier2_scan_min_age_boundary_and_future_mtime() {
    // Use real LocalAppData\Temp\__sweeper_scan_gate_e_* area (allowed for test content; read-only scan).
    let local = sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData");
    let base = local.join("Temp").join("__sweeper_scan_gate_e_age");
    let _ = std::fs::create_dir_all(&base);
    let old = base.join("old.txt");
    let recent = base.join("recent.txt");
    let future = base.join("future.txt");
    let boundary = base.join("boundary.txt");
    fs::write(&old, b"old").unwrap();
    fs::write(&recent, b"recent").unwrap();
    fs::write(&future, b"future").unwrap();
    fs::write(&boundary, b"boundary").unwrap();
    let now = SystemTime::now();
    set_file_mtime(&old, now - Duration::from_secs(26 * 3600));
    set_file_mtime(&recent, now - Duration::from_secs(23 * 3600));
    set_file_mtime(&boundary, now - Duration::from_secs(24 * 3600));
    set_file_mtime(&future, now + Duration::from_secs(2 * 3600));

    let target = make_target(
        "gate-e-temp-age",
        KnownFolder::LocalAppData,
        "Temp\\__sweeper_scan_gate_e_age",
        24,
    );
    let report = scanner::scan_targets(vec![target], 24).expect("scan");
    assert_eq!(report.targets.len(), 1);
    let t = &report.targets[0];
    assert!(t.scanned, "status: {:?}", t.status_reason);
    assert_eq!(t.file_count, 4);
    // future treated as recent/ineligible; boundary (>=24h) eligible; old eligible; recent ineligible
    assert_eq!(
        t.eligible_count,
        2,
        "eligible items: {:?}",
        t.items
            .iter()
            .map(|i| (&i.path, i.age_eligible, i.mtime_unix_secs))
            .collect::<Vec<_>>()
    );
    assert!(t.eligible_bytes >= 7);

    cleanup_file_best_effort(&old);
    cleanup_file_best_effort(&recent);
    cleanup_file_best_effort(&future);
    cleanup_file_best_effort(&boundary);
    cleanup_dir_best_effort(&base);
}

#[test]
fn wildcard_intermediate_segment_expansion_scans_matching_dirs_only() {
    let local = sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData");
    let base = local.join("Temp").join("__sweeper_scan_gate_e_wc");
    cleanup_dir_best_effort(&base);
    let a = base.join("Profile A").join("caches");
    let b = base.join("Profile B").join("caches");
    let c = base.join("Other").join("caches");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::create_dir_all(&c).unwrap();
    fs::write(a.join("a.txt"), b"aaa").unwrap();
    fs::write(b.join("b.txt"), b"bbb").unwrap();
    fs::write(c.join("c.txt"), b"ccc").unwrap();
    let now = SystemTime::now();
    for p in [a.join("a.txt"), b.join("b.txt"), c.join("c.txt")] {
        set_file_mtime(&p, now - Duration::from_secs(48 * 3600));
    }

    let target = make_target(
        "gate-e-wc",
        KnownFolder::LocalAppData,
        "Temp\\__sweeper_scan_gate_e_wc\\Profile *\\caches",
        24,
    );
    let report = scanner::scan_targets(vec![target], 24).expect("scan");
    let t = &report.targets[0];
    assert!(t.scanned, "status: {:?}", t.status_reason);
    assert_eq!(
        t.resolved_paths.len(),
        2,
        "resolved: {:?}",
        t.resolved_paths
    );
    assert_eq!(
        t.file_count,
        2,
        "files: {:?}",
        t.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    assert!(t.items.iter().all(|i| i.path.contains("\\Profile ")));
    assert!(!t.items.iter().any(|i| i.path.contains("\\Other\\")));

    cleanup_dir_best_effort(&base);
}

#[test]
fn redirected_temp_guard_disables_unsafe_or_identical_targets() {
    // We can't reliably mutate real %TEMP% safely here, but we can validate guard behavior indirectly:
    // evaluate by scanning embedded targets should never mark redirected-temp as safe when identical to user-temp.
    let report = scanner::scan_tier1_tier2(24).expect("scan");
    if let Some(rt) = report
        .targets
        .iter()
        .find(|t| t.target_id == "redirected-temp")
    {
        // On typical systems %TEMP% == LocalAppData\Temp so redirected-temp must be disabled.
        // If different and safe, it may be scanned; but must never be unsafe.
        if let Some(status) = &rt.status_reason {
            assert!(status.contains("redirected-temp disabled") || status.contains("not present"));
        } else {
            // scanned: ensure resolved roots are not protected and depth>=3
            for p in &rt.resolved_paths {
                let c = CanonicalPath::from_existing_path(p)
                    .unwrap()
                    .into_canonical();
                assert!(c.segment_count() >= 3);
                assert!(!policy::is_subtree_protected(&c));
                assert!(!policy::is_protected_container_root(&c));
            }
        }
    }
}

#[test]
#[cfg(windows)]
fn junction_symlink_inside_target_skipped_and_canary_outside_intact() {
    let local = sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData");
    let base = local.join("Temp").join("__sweeper_scan_gate_e_link");
    cleanup_dir_best_effort(&base);
    let target_dir = base.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("safe.txt"), b"safe").unwrap();

    let canary = CanaryTree::seed_external("gate-e-scan").expect("seed canary");
    let link = target_dir.join("escape-junc");
    sweeper_testkit::create_junction(&link, canary.root()).expect("create junction");
    let now = SystemTime::now();
    set_file_mtime(
        &target_dir.join("safe.txt"),
        now - Duration::from_secs(48 * 3600),
    );

    let target = make_target(
        "gate-e-link",
        KnownFolder::LocalAppData,
        "Temp\\__sweeper_scan_gate_e_link\\target",
        24,
    );
    let report = scanner::scan_targets(vec![target], 24).expect("scan");
    let t = &report.targets[0];
    assert!(t.scanned, "status: {:?}", t.status_reason);
    assert_eq!(
        t.file_count, 1,
        "should only count safe.txt, got {:?}",
        t.items
    );
    assert!(t.skipped_count >= 1);
    assert!(t.skipped_reasons.contains_key("LinkPoint"));
    canary
        .verify()
        .expect("canary must remain intact (read-only scan)");

    cleanup_dir_best_effort(&base);
}

#[test]
fn deep_tree_iterative_walk_handles_1000_levels_without_stack_overflow() {
    let local = sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData");
    let base = local.join("Temp").join("__sweeper_scan_gate_e_deep");
    let mut cur = base.clone();
    // Keep path lengths reasonable on Windows by using short segment names.
    fs::create_dir_all(&cur).unwrap();
    for _ in 0..1000 {
        cur.push("d");
        fs::create_dir_all(&cur).unwrap();
    }
    let leaf = cur.join("leaf.txt");
    fs::write(&leaf, b"x").unwrap();
    let now = SystemTime::now();
    set_file_mtime(&leaf, now - Duration::from_secs(48 * 3600));

    let target = make_target(
        "gate-e-deep",
        KnownFolder::LocalAppData,
        "Temp\\__sweeper_scan_gate_e_deep",
        24,
    );
    let report = scanner::scan_targets(vec![target], 24).expect("scan");
    let t = &report.targets[0];
    assert!(t.scanned, "status: {:?}", t.status_reason);
    assert_eq!(t.file_count, 1);

    cleanup_dir_best_effort(&base);
}

#[test]
fn real_machine_gate_e_read_only_verification() {
    // Read-only scan of embedded Tier 1 & 2 targets on this machine.
    let report = scanner::scan_tier1_tier2(24).expect("scan embedded targets");
    let violations = scanner::gate_e_violations(&report);
    assert!(
        violations.is_empty(),
        "Gate E violations:\n{}",
        violations.join("\n")
    );

    // Also assert scanned resolved roots never target protected container roots/subtree-protected paths.
    for t in report.targets.iter().filter(|t| t.scanned) {
        for p in &t.resolved_paths {
            let c = CanonicalPath::from_existing_path(p)
                .unwrap()
                .into_canonical();
            assert!(
                !policy::is_protected_container_root(&c),
                "container root targeted: {}",
                c
            );
            assert!(
                !policy::is_subtree_protected(&c),
                "subtree protected targeted: {}",
                c
            );
            assert!(
                !policy::is_ancestor_of_subtree_protected(&c),
                "ancestor of protected: {}",
                c
            );
        }
    }
}
