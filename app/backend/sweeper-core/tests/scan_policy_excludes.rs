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

fn policy_temp_base() -> std::path::PathBuf {
    sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData")
        .join("Temp")
}

fn setup_policy_fixture(base: &Path) {
    fs::write(base.join("swap.vhdx"), b"fakevhdx").unwrap();
    let git_dir = base.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("HEAD"), b"ref\n").unwrap();
    let nm = base.join("node_modules").join("pkg");
    fs::create_dir_all(&nm).unwrap();
    fs::write(nm.join("index.js"), b"module.exports={}").unwrap();
    fs::write(base.join("normal.txt"), b"x").unwrap();
    set_old(&base.join("normal.txt"));
}

fn policy_target(relative_subpath: &str) -> TargetDefinition {
    TargetDefinition {
        id: "policy-exclude-test".to_string(),
        name: "policy-exclude-test".to_string(),
        tier: Tier::Tier1,
        risk: Risk::Safe,
        known_folder: KnownFolder::LocalAppData,
        relative_subpath: relative_subpath.to_string(),
        scope: Scope::Contents,
        min_age_hours: 0,
        description: "test".to_string(),
        requires_elevation: false,
    }
}

fn is_excluded_path(path: &str) -> bool {
    let lp = path.to_ascii_lowercase();
    lp.ends_with(".vhdx") || lp.ends_with(".vhd") || lp.contains("\\.git\\")
}

fn has_node_modules_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains("\\node_modules\\")
}

fn assert_policy_scan(report: &scanner::ScanReport) {
    let t = &report.targets[0];
    assert!(t.items.iter().all(|it| !is_excluded_path(&it.path)));
    assert!(t.items.iter().all(|it| !has_node_modules_path(&it.path)));
    assert!(t.items.iter().any(|it| it.path.ends_with("normal.txt")));
    let count = t
        .skipped_reasons
        .get("PolicyExcluded")
        .copied()
        .unwrap_or(0);
    assert!(count >= 3);
}

#[test]
fn policy_excludes_vhdx_git_and_node_modules_in_scanned_target() {
    let temp_base = policy_temp_base();
    let fixture_dir = tempfile::Builder::new()
        .prefix("sweeper-policy-exclude-")
        .tempdir_in(&temp_base)
        .unwrap();
    setup_policy_fixture(fixture_dir.path());
    let name = fixture_dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let relative_subpath = format!("Temp\\{name}");
    let report = scanner::scan_targets(vec![policy_target(&relative_subpath)], 0).expect("scan");
    assert_policy_scan(&report);
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
