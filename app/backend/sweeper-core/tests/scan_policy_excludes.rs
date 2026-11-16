#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sweeper_core::scanner;
use sweeper_knowledge::{KnownFolder, Risk, Scope, TargetDefinition, Tier};

fn set_old(path: &Path) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(48 * 3600);
    let ft = sweeper_testkit::filetime_from_unix_secs(secs);
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

fn setup_policy_fixture(base: &Path) {
    fs::write(base.join("swap.vhdx"), b"fakevhdx").unwrap();
    let git_dir = base.join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::write(base.join("normal.txt"), b"x").unwrap();
    set_old(&base.join("normal.txt"));
}

fn check_policy_excludes_vhdx_and_git_in_scanned_target() {
    let local = sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::LocalAppData)
        .expect("resolve LocalAppData");
    let base = local
        .join("Temp")
        .join("__sweeper_scan_policy_excludes_test");
    let _ = std::process::Command::new("cmd")
        .args(["/C", "rmdir", "/S", "/Q", &base.to_string_lossy()])
        .status();
    fs::create_dir_all(&base).unwrap();
    setup_policy_fixture(&base);
    let target = TargetDefinition {
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
    };
    let report = scanner::scan_targets(vec![target], 0).expect("scan");
    let t = &report.targets[0];
    assert!(t.items.iter().all(|it| {
        let lp = it.path.to_ascii_lowercase();
        !lp.ends_with(".vhdx") && !lp.ends_with(".vhd") && !lp.contains("\\.git\\")
    }));
    assert!(
        t.skipped_reasons
            .get("PolicyExcluded")
            .copied()
            .unwrap_or(0)
            >= 2
    );
    let _ = std::process::Command::new("cmd")
        .args(["/C", "rmdir", "/S", "/Q", &base.to_string_lossy()])
        .status();
}

#[test]
fn policy_excludes_vhdx_and_git_in_scanned_target() {
    check_policy_excludes_vhdx_and_git_in_scanned_target();
}
