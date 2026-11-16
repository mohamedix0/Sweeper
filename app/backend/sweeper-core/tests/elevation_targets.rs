#![cfg(windows)]

use sweeper_core::scanner::{self, ScanSettingsOptions};
use sweeper_core::settings::SweeperSettings;
use sweeper_core::ScanElevationContext;

#[test]
fn elevation_settings_default_and_sanitize() {
    let s = SweeperSettings::default();
    assert!(!s.enable_elevation_targets);

    let parsed2 = SweeperSettings::parse_untrusted_json(r#"{"enable_elevation_targets": true}"#);
    assert!(parsed2.settings.enable_elevation_targets);
    assert!(parsed2.warnings.is_empty());

    let parsed3 = SweeperSettings::parse_untrusted_json(r#"{"enableElevationTargets": true}"#);
    assert!(parsed3.settings.enable_elevation_targets);

    let parsed4 = SweeperSettings::parse_untrusted_json(r#"{"enable_elevation_targets": "yes"}"#);
    assert!(!parsed4.settings.enable_elevation_targets);
    assert!(parsed4
        .warnings
        .iter()
        .any(|w| w.contains("enable_elevation_targets invalid")));
}

#[test]
fn elevation_target_exclusion_respects_elevation_and_setting_flags() {
    const EXCLUDED: [&str; 4] = [
        "windows-temp",
        "windows-update-download-cache",
        "windows-prefetch",
        "programdata-package-cache",
    ];
    for (label, enable_elevation_targets, process_elevated) in [
        ("non-elevated+enabled", true, false),
        ("elevated+disabled", false, true),
    ] {
        let settings = SweeperSettings {
            enable_tier1: false,
            enable_tier2: false,
            enable_tier3: true,
            enable_elevation_targets,
            ..SweeperSettings::default()
        };
        let report = scanner::scan_with_settings_options_and_elevation_with_progress(
            &settings,
            ScanSettingsOptions::default(),
            ScanElevationContext { process_elevated },
            scanner::ScanRunControl::default(),
        )
        .unwrap_or_else(|e| panic!("{label}: scan failed: {e}"));
        for target in &report.targets {
            assert!(
                !EXCLUDED.contains(&target.target_id.as_str()),
                "{label}: unexpected {}",
                target.target_id
            );
        }
    }
}

fn sandbox_in_target() -> sweeper_testkit::Sandbox {
    use std::fs;
    use std::path::PathBuf;
    let base = if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
        PathBuf::from(v).join("sweeper-test-sandboxes")
    } else {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .join("../../target/sweeper-test-sandboxes")
            .components()
            .collect::<PathBuf>()
    };
    let _ = fs::create_dir_all(&base);
    let canon = std::fs::canonicalize(&base).unwrap_or(base);
    sweeper_testkit::Sandbox::new_in(canon)
}

fn set_mtime_path(path: &std::path::Path, unix_secs: u64) {
    let ft = sweeper_testkit::filetime_from_unix_secs(unix_secs);
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

fn strip_extended_prefix(p: std::path::PathBuf) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return std::path::PathBuf::from(rest.to_string());
    }
    p
}

fn canon_str_path(p: &std::path::Path) -> String {
    use sweeper_core::paths::CanonicalPath;
    let stripped = strip_extended_prefix(p.to_path_buf());
    CanonicalPath::from_existing_path(&stripped.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

struct ElevSandboxFixture {
    sb: sweeper_testkit::Sandbox,
    elevated_root: std::path::PathBuf,
    old_file: std::path::PathBuf,
    recent_file: std::path::PathBuf,
    canary_file: std::path::PathBuf,
    link_path: std::path::PathBuf,
}

fn create_elev_fixture() -> ElevSandboxFixture {
    use std::fs;
    let sb = sandbox_in_target();
    let elevated_root = sb.create_dir(r"elev\windows-temp").unwrap();
    let outside_canary_parent = sb.create_dir(r"outside\canary").unwrap();
    assert!(!elevated_root
        .to_string_lossy()
        .to_ascii_uppercase()
        .starts_with(r"C:\WINDOWS"));
    assert!(!elevated_root
        .to_string_lossy()
        .to_ascii_uppercase()
        .starts_with(r"C:\PROGRAMDATA"));

    let old_file = elevated_root.join("old-eligible.tmp");
    fs::write(&old_file, b"old-eligible-bytes").unwrap();
    let old_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(48 * 3600);
    set_mtime_path(&old_file, old_secs);

    let recent_file = elevated_root.join("recent-skip.tmp");
    fs::write(&recent_file, b"recent").unwrap();
    let recent_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(3600);
    set_mtime_path(&recent_file, recent_secs);

    let canary_file = outside_canary_parent.join("canary.txt");
    fs::write(&canary_file, b"canary-outside").unwrap();
    let link_path = elevated_root.join("escape-link");
    sweeper_testkit::create_junction(&link_path, &outside_canary_parent).unwrap();

    ElevSandboxFixture {
        sb,
        elevated_root,
        old_file,
        recent_file,
        canary_file,
        link_path,
    }
}

fn run_elev_scan(root: &std::path::Path) -> sweeper_core::scanner::ScanReport {
    use std::collections::BTreeMap;
    let mut overrides = BTreeMap::new();
    overrides.insert("windows-temp".to_string(), canon_str_path(root));
    let settings = SweeperSettings {
        enable_tier1: true,
        enable_tier2: false,
        enable_tier3: true,
        enable_elevation_targets: true,
        min_age_hours: 24,
        ..SweeperSettings::default()
    };
    scanner::scan_with_settings_options_and_elevation_with_progress(
        &settings,
        ScanSettingsOptions {
            elevation_target_root_overrides: overrides,
            ..ScanSettingsOptions::default()
        },
        ScanElevationContext {
            process_elevated: true,
        },
        scanner::ScanRunControl::default(),
    )
    .unwrap()
}

fn assert_tier1_safe_unchanged(report: &sweeper_core::scanner::ScanReport) {
    use sweeper_knowledge::{Risk, Tier};
    let found = report.targets.iter().any(|t| t.target_id == "user-temp");
    assert!(found, "expected user-temp target present");
    let t = report
        .targets
        .iter()
        .find(|t| t.target_id == "user-temp")
        .unwrap();
    assert_eq!(t.tier, Tier::Tier1);
    assert_eq!(t.risk, Risk::Safe);
}

fn get_elev_target(
    report: &sweeper_core::scanner::ScanReport,
) -> &sweeper_core::scanner::TargetScanResult {
    use sweeper_knowledge::{Risk, Tier};
    let elev = report
        .targets
        .iter()
        .find(|t| t.target_id == "windows-temp")
        .expect("elevation target discovered via override");
    assert!(elev.scanned, "status: {:?}", elev.status_reason);
    assert_eq!(elev.tier, Tier::Tier3);
    assert_eq!(elev.risk, Risk::Caution);
    elev
}

fn assert_elev_scan_items(
    elev: &sweeper_core::scanner::TargetScanResult,
    old_file: &std::path::Path,
    recent_file: &std::path::Path,
) {
    let old_name = old_file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_ascii_lowercase();
    let recent_name = recent_file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_ascii_lowercase();
    let old_item = elev
        .items
        .iter()
        .find(|i| i.path.to_ascii_lowercase().contains(&old_name))
        .expect("eligible old file discovered");
    assert!(old_item.age_eligible, "old file should be eligible");
    assert!(
        !elev
            .items
            .iter()
            .any(|i| i.path.to_ascii_lowercase().contains(&recent_name) && i.age_eligible),
        "recent file must be ineligible: {:?}",
        elev.items
    );
}

fn assert_linkpoint_skipped(elev: &sweeper_core::scanner::TargetScanResult) {
    let has_reason = elev.skipped_reasons.contains_key("LinkPoint");
    let has_count = elev.skipped_count >= 1;
    assert!(has_reason || has_count, "reparse link should be skipped");
}

fn build_validated_items(
    elev: &sweeper_core::scanner::TargetScanResult,
    root_canon: &str,
) -> Vec<sweeper_core::remover::ValidatedItem> {
    use sweeper_core::remover::ValidatedItem;
    elev.items
        .iter()
        .filter(|i| i.age_eligible)
        .map(|i| {
            ValidatedItem::from_allowlisted_scan(
                i.id.clone(),
                "windows-temp",
                root_canon.to_string(),
                i.path.clone(),
                i.size,
                Some(i.mtime_unix_secs),
            )
            .unwrap()
        })
        .collect()
}

fn execute_elev_clean(
    sb: &sweeper_testkit::Sandbox,
    elev: &sweeper_core::scanner::TargetScanResult,
    validated: Vec<sweeper_core::remover::ValidatedItem>,
) -> sweeper_core::remover::CleanReport {
    use std::time::{Duration, SystemTime};
    use sweeper_core::remover::{self, CleanOptions};
    let journal_dir = sb.create_dir("journal-elev").unwrap();
    remover::execute_clean(
        validated,
        CleanOptions {
            preview_estimated_bytes: elev.eligible_bytes.max(1),
            min_age: Duration::from_secs(24 * 3600),
            now: SystemTime::now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap()
}

fn assert_post_clean_state(clean: &sweeper_core::remover::CleanReport, fx: &ElevSandboxFixture) {
    assert!(!clean.aborted, "clean aborted: {:?}", clean.abort_reason);
    for (path, expected_exists, msg) in [
        (
            fx.elevated_root.as_path(),
            true,
            "target root must remain intact",
        ),
        (
            fx.old_file.as_path(),
            false,
            "eligible old file should be cleaned",
        ),
        (fx.recent_file.as_path(), true, "recent file should remain"),
        (fx.canary_file.as_path(), true, "outside canary must remain"),
        (fx.link_path.as_path(), true, "link entry may remain"),
    ] {
        assert_eq!(path.exists(), expected_exists, "{msg}");
    }
}

#[test]
fn elevated_scan_discovers_and_cleans_sandboxed_elevation_target_contents_only() {
    let fx = create_elev_fixture();
    let report = run_elev_scan(&fx.elevated_root);
    assert_tier1_safe_unchanged(&report);

    let elev = get_elev_target(&report);
    assert_elev_scan_items(elev, &fx.old_file, &fx.recent_file);
    assert_linkpoint_skipped(elev);

    let root_canon = canon_str_path(&fx.elevated_root);
    let validated = build_validated_items(elev, &root_canon);
    assert!(!validated.is_empty(), "expected eligible items");

    let clean = execute_elev_clean(&fx.sb, elev, validated);
    assert_post_clean_state(&clean, &fx);
}
