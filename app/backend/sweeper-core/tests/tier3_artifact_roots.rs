#![cfg(windows)]

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sweeper_core::scanner::{self, ScanSettingsOptions};
use sweeper_core::settings::SweeperSettings;
use sweeper_testkit::Sandbox;

fn unix_secs_days_ago(days: u64) -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(days * 24 * 3600)
}

fn set_mtime_days_ago(path: &Path, age_days: u64) {
    let ft = sweeper_testkit::filetime_from_unix_secs(unix_secs_days_ago(age_days));
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

fn set_old(path: &Path) {
    set_mtime_days_ago(path, 2);
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn tier3_artifact_promoted_to_search_root_is_classified_and_pruned() {
    let sb = Sandbox::new();
    // Mirrors a user profile that directly contains `node_modules` beside an old
    // `package-lock.json`. Tier 3 root discovery promotes the artifact itself to a search
    // root; the walker must classify that root and prune its dependency tree.
    let profile = sb.create_dir("profile").unwrap();
    write_file(&profile.join("package-lock.json"), b"{}");
    set_mtime_days_ago(&profile.join("package-lock.json"), 45);

    let node_modules = profile.join("node_modules");
    fs::create_dir_all(node_modules.join("pkg")).unwrap();
    write_file(
        &node_modules.join("pkg").join("index.js"),
        b"console.log('x')",
    );
    set_old(&node_modules.join("pkg").join("index.js"));

    // A nested `target/` with an old sibling `Cargo.toml` inside the dependency tree. If the
    // walker enumerated the tree instead of pruning it, this would be reported as a separate
    // stale rust target - its absence proves the dependency tree was pruned.
    let nested_foo = node_modules.join("foo");
    fs::create_dir_all(nested_foo.join("target")).unwrap();
    write_file(
        &nested_foo.join("Cargo.toml"),
        b"[package]\nname='nested'\nversion='0.0.0'\n",
    );
    write_file(&nested_foo.join("target").join("artifact.bin"), b"12345");
    set_mtime_days_ago(&nested_foo.join("Cargo.toml"), 45);

    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        stale_project_days: 30,
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        // Discovery promotes `profile/node_modules` itself to a search root.
        tier3_project_search_roots: vec![node_modules.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();

    let stale_nm = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-node-modules")
        .expect("stale node_modules result");
    assert!(
        stale_nm.scanned,
        "the promoted artifact root must be classified: {:?}",
        stale_nm.status_reason
    );
    assert_eq!(
        stale_nm.items.len(),
        1,
        "the promoted node_modules root must be one stale artifact item: {:?}",
        stale_nm.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    assert!(
        stale_nm.items[0]
            .path
            .to_ascii_lowercase()
            .ends_with("\\node_modules"),
        "classified item must be the node_modules artifact: {:?}",
        stale_nm.items[0].path
    );

    let stale_rust = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-rust-target")
        .expect("stale rust target result");
    assert_eq!(
        stale_rust.items.len(),
        0,
        "the nested target inside the pruned dependency tree must not be enumerated: {:?}",
        stale_rust.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
}

#[test]
fn tier3_non_artifact_target_root_is_traversed_for_nested_artifacts() {
    let sb = Sandbox::new();
    // A search root merely named `target` is not automatically a Rust build artifact: its
    // parent has no Cargo.toml. The walker must traverse it so nested artifacts are found.
    let container = sb.create_dir("container").unwrap();
    let target_root = container.join("target");
    fs::create_dir_all(&target_root).unwrap();
    assert!(
        !container.join("Cargo.toml").exists(),
        "fixture precondition: no build marker beside the target-named root"
    );

    let project = target_root.join("nested-project");
    let nested_target = project.join("target");
    fs::create_dir_all(&nested_target).unwrap();
    write_file(
        &project.join("Cargo.toml"),
        b"[package]\nname='nested'\nversion='0.0.0'\n",
    );
    write_file(&nested_target.join("artifact.bin"), b"12345");
    set_mtime_days_ago(&project.join("Cargo.toml"), 45);
    set_old(&nested_target);

    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        stale_project_days: 30,
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: vec![target_root.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();

    let stale_rust = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-rust-target")
        .expect("stale rust target result");
    assert_eq!(
        stale_rust.items.len(),
        1,
        "the nested stale target under a non-artifact root must be discovered: {:?}",
        stale_rust.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    assert!(
        stale_rust.items[0]
            .path
            .to_ascii_lowercase()
            .ends_with("\\nested-project\\target"),
        "the discovered item must be the nested rust target: {:?}",
        stale_rust.items[0].path
    );
}
