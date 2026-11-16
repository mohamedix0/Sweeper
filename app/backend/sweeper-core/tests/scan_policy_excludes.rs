#![cfg(windows)]

use std::fs;
use std::path::Path;

use sweeper_core::scanner::{self, ScanSettingsOptions};
use sweeper_core::settings::SweeperSettings;
use sweeper_knowledge::{KnownFolder, Risk, Scope, TargetDefinition, Tier};

fn old_secs_ago(secs: u64) -> std::time::SystemTime {
    std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(secs))
        .unwrap()
}

fn set_old(path: &Path) {
    let secs = old_secs_ago(48 * 3600)
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let ft = sweeper_testkit::filetime_from_unix_secs(secs);
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

fn policy_test_base() -> std::path::PathBuf {
    let local = sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData");
    local
        .join("Temp")
        .join("__sweeper_scan_policy_excludes_test")
}

fn reset_dir(base: &Path) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "rmdir", "/S", "/Q", &base.to_string_lossy()])
        .status();
    fs::create_dir_all(base).unwrap();
}

fn setup_policy_fixture(base: &Path) {
    fs::write(base.join("swap.vhdx"), b"fakevhdx").unwrap();
    let git_dir = base.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::write(base.join("normal.txt"), b"x").unwrap();
    set_old(&base.join("normal.txt"));
}

fn policy_target() -> TargetDefinition {
    TargetDefinition {
        id: "policy-exclude-test".to_string(),
        name: "policy-exclude-test".to_string(),
        tier: Tier::Tier1,
        risk: Risk::Safe,
        known_folder: KnownFolder::LocalAppData,
        relative_subpath: "Temp\\__sweeper_scan_policy_excludes_test".to_string(),
        scope: Scope::Contents,
        min_age_hours: 0,
        description: "test".to_string(),
        requires_elevation: false,
    }
}

fn is_excluded_path(path: &str) -> bool {
    let lp = path.to_ascii_lowercase();
    if lp.ends_with(".vhdx") {
        return true;
    }
    if lp.ends_with(".vhd") {
        return true;
    }
    lp.contains("\\.git\\")
}

fn assert_policy_scan(report: &scanner::ScanReport) {
    let t = &report.targets[0];
    let excluded = t.items.iter().all(|it| !is_excluded_path(&it.path));
    assert!(excluded);
    let count = t
        .skipped_reasons
        .get("PolicyExcluded")
        .copied()
        .unwrap_or(0);
    assert!(count >= 2);
}

#[test]
fn policy_excludes_vhdx_and_git_in_scanned_target() {
    let base = policy_test_base();
    reset_dir(&base);
    setup_policy_fixture(&base);
    let report = scanner::scan_targets(vec![policy_target()], 0).expect("scan");
    assert_policy_scan(&report);
    let _ = std::process::Command::new("cmd")
        .args(["/C", "rmdir", "/S", "/Q", &base.to_string_lossy()])
        .status();
}

#[test]
fn custom_tier3_empty_roots_discovers_default_roots_gracefully() {
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: Vec::new(),
        recycle_bin_roots: Vec::new(),
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).expect("scan");
    assert!(report.targets.iter().all(|t| t.tier == Tier::Tier3));
}
