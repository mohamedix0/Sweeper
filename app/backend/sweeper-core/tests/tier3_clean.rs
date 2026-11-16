#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sweeper_core::remover::{self, CleanOptions, ValidatedItem};
use sweeper_core::scanner::{self, ScanSettingsOptions};
use sweeper_core::settings::SweeperSettings;
use sweeper_testkit::Sandbox;

fn old_mtime_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(48 * 3600)
}

fn unix_secs_days_ago(days: u64) -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(days * 24 * 3600)
}

fn set_old(path: &Path) {
    let ft = sweeper_testkit::filetime_from_unix_secs(old_mtime_secs());
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

fn set_mtime_days_ago(path: &Path, age_days: u64) {
    let ft = sweeper_testkit::filetime_from_unix_secs(unix_secs_days_ago(age_days));
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

fn write_file(path: &Path, contents: &[u8]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn write_old_file(path: &Path, age_days: u64) {
    write_file(path, b"x");
    set_mtime_days_ago(path, age_days);
}

fn canon_str(p: &Path) -> String {
    sweeper_core::paths::CanonicalPath::from_existing_path(&p.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

fn clean_diagnostics(clean: &remover::CleanReport, journal_dir: &Path) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "clean.aborted={}\nclean.abort_reason={:?}\nclean.bytes_freed={}\nclean.skipped_count={}\nclean.items={:#?}\n",
        clean.aborted, clean.abort_reason, clean.bytes_freed, clean.skipped_count, clean.items
    ));
    out.push_str("journal_files:\n");
    let log_dir = sweeper_core::journal::resolve_log_dir(journal_dir);
    match fs::read_dir(&log_dir) {
        Ok(entries) => {
            let mut paths: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
                .collect();
            paths.sort();
            if paths.is_empty() {
                out.push_str("  <none>\n");
            }
            for p in paths {
                out.push_str(&format!("--- {} ---\n", p.display()));
                match fs::read_to_string(&p) {
                    Ok(s) => {
                        if s.is_empty() {
                            out.push_str("<empty>\n");
                        } else {
                            out.push_str(&s);
                            if !s.ends_with('\n') {
                                out.push('\n');
                            }
                        }
                    }
                    Err(e) => out.push_str(&format!("<read error: {e}>\n")),
                }
            }
        }
        Err(e) => out.push_str(&format!("  <read_dir error: {e}>\n")),
    }
    out
}

fn journal_has_locked_skip(journal_dir: &Path) -> bool {
    let log_dir = sweeper_core::journal::resolve_log_dir(journal_dir);
    let Ok(entries) = fs::read_dir(log_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        for line in contents.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("record").and_then(|r| r.as_str()) == Some("outcome")
                && v.get("status").and_then(|s| s.as_str()) == Some("Skipped")
                && v.get("reason")
                    .and_then(|r| r.as_str())
                    .is_some_and(|r| r.to_ascii_lowercase().contains("locked"))
            {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::type_complexity)]
fn setup_tier3_scan_fixture(
    sb: &Sandbox,
) -> (
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let search_root = sb.create_dir("projects").unwrap();
    let rust_proj = search_root.join("rustproj");
    let rust_target = rust_proj.join("target");
    fs::create_dir_all(&rust_target).unwrap();
    write_file(
        &rust_proj.join("Cargo.toml"),
        b"[package]\nname='x'\nversion='0.0.0'\n",
    );
    write_file(&rust_target.join("artifact.bin"), b"12345");
    set_mtime_days_ago(&rust_proj.join("Cargo.toml"), 45);
    set_old(&rust_target.join("artifact.bin"));
    set_old(&rust_target);

    let rust_fresh = search_root.join("rustfresh");
    let rust_fresh_target = rust_fresh.join("target");
    fs::create_dir_all(&rust_fresh_target).unwrap();
    write_file(
        &rust_fresh.join("Cargo.toml"),
        b"[package]\nname='y'\nversion='0.0.0'\n",
    );
    write_file(&rust_fresh_target.join("artifact.bin"), b"12345");
    sweeper_testkit::set_last_write_time(
        &rust_fresh.join("Cargo.toml"),
        sweeper_testkit::filetime_from_unix_secs(unix_secs_days_ago(0).saturating_sub(2 * 3600)),
    )
    .unwrap();

    let orphan_target = search_root.join("orphan").join("target");
    fs::create_dir_all(&orphan_target).unwrap();
    write_file(&orphan_target.join("artifact.bin"), b"12345");

    let node_proj = search_root.join("nodeproj");
    let node_modules = node_proj.join("node_modules");
    fs::create_dir_all(node_modules.join("pkg")).unwrap();
    write_file(&node_proj.join("package-lock.json"), b"{}");
    write_file(
        &node_modules.join("pkg").join("index.js"),
        b"console.log('x')",
    );
    set_mtime_days_ago(&node_proj.join("package-lock.json"), 45);
    set_old(&node_modules.join("pkg").join("index.js"));
    set_old(&node_modules.join("pkg"));
    set_old(&node_modules);

    let node_fresh = search_root.join("nodefresh");
    let node_fresh_nm = node_fresh.join("node_modules");
    fs::create_dir_all(&node_fresh_nm).unwrap();
    write_file(&node_fresh.join("pnpm-lock.yaml"), b"lockfileVersion: 9\n");
    write_file(&node_fresh_nm.join("x.js"), b"x");
    sweeper_testkit::set_last_write_time(
        &node_fresh.join("pnpm-lock.yaml"),
        sweeper_testkit::filetime_from_unix_secs(unix_secs_days_ago(0).saturating_sub(2 * 3600)),
    )
    .unwrap();

    let node_orphan = search_root.join("nodeorphan");
    let node_orphan_nm = node_orphan.join("node_modules");
    fs::create_dir_all(&node_orphan_nm).unwrap();
    write_file(&node_orphan_nm.join("x.js"), b"x");

    let recycle = sb.create_dir("recycle_sid").unwrap();
    write_old_file(&recycle.join("$IABC.txt"), 2);
    fs::create_dir_all(recycle.join("dir")).unwrap();
    write_old_file(&recycle.join("dir").join("inside.txt"), 2);
    set_old(&recycle.join("dir"));

    let rec_file = recycle.join("$IABC.txt");
    let rec_dir = recycle.join("dir");
    (
        search_root,
        rust_target,
        rust_fresh_target,
        orphan_target,
        node_modules,
        node_fresh_nm,
        node_orphan_nm,
        recycle,
        rec_file,
        rec_dir,
    )
}

#[allow(clippy::too_many_arguments)]
fn assert_tier3_clean_result(
    clean: &remover::CleanReport,
    diag: &str,
    recycle: &std::path::Path,
    rec_file: &std::path::Path,
    rec_dir: &std::path::Path,
    rust_target: &std::path::Path,
    node_modules: &std::path::Path,
    rust_fresh_target: &std::path::Path,
    orphan_target: &std::path::Path,
    node_fresh_nm: &std::path::Path,
    node_orphan_nm: &std::path::Path,
) {
    assert!(
        !clean.aborted,
        "tier 3 clean aborted: {:?}\n{}",
        clean.abort_reason, diag
    );
    assert!(
        recycle.exists() && !rec_file.exists() && !rec_dir.exists(),
        "recycle preserved/contents removed\n{diag}"
    );
    assert!(
        !rust_target.exists() && !node_modules.exists(),
        "stale dirs removed\n{diag}"
    );
    assert!(
        rust_fresh_target.exists()
            && orphan_target.exists()
            && node_fresh_nm.exists()
            && node_orphan_nm.exists(),
        "fresh/orphan remain\n{diag}"
    );
}

#[rustfmt::skip]
#[test]
fn tier3_clean_removes_stale_dirs_and_recycle_contents_only() {
    let sb = Sandbox::new();
    let (search_root, rust_target, rust_fresh_target, orphan_target, node_modules, node_fresh_nm, node_orphan_nm, recycle, rec_file, rec_dir) = setup_tier3_scan_fixture(&sb);
    let settings = SweeperSettings { enable_tier1: false, enable_tier2: false, enable_tier3: true, stale_project_days: 30, min_age_hours: 0, ..SweeperSettings::default() };
    let opts = ScanSettingsOptions { tier3_project_search_roots: vec![search_root.to_string_lossy().to_string()], recycle_bin_roots: vec![recycle.to_string_lossy().to_string()], ..Default::default() };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();
    let (stale_rust, stale_nm, rb) = (report.targets.iter().find(|t| t.target_id == "stale-rust-target").unwrap(), report.targets.iter().find(|t| t.target_id == "stale-node-modules").unwrap(), report.targets.iter().find(|t| t.target_id == "recycle-bin").unwrap());
    let build_validated = || vec![ValidatedItem::from_tier3_recycle_bin_root("rb", canon_str(&recycle), rb.total_bytes).unwrap(), ValidatedItem::from_tier3_project_directory("rust-target", "stale-rust-target", canon_str(&rust_target), stale_rust.total_bytes, Some(old_mtime_secs())).unwrap(), ValidatedItem::from_tier3_project_directory("node-modules", "stale-node-modules", canon_str(&node_modules), stale_nm.total_bytes, Some(old_mtime_secs())).unwrap()];
    let journal_dir = sb.create_dir("journal").unwrap();
    let clean_opts = || CleanOptions { preview_estimated_bytes: rb.total_bytes + stale_rust.total_bytes + stale_nm.total_bytes + 1024, min_age: Duration::from_secs(0), now: SystemTime::now(), journal_base_dir: Some(journal_dir.clone()) };
    let mut clean = remover::execute_clean(build_validated(), clean_opts()).unwrap();
    for attempt in 0..3 {
        if !rec_file.exists() && !rec_dir.exists() && !rust_target.exists() && !node_modules.exists() { break; }
        if attempt < 2 && journal_has_locked_skip(&journal_dir) { std::thread::sleep(Duration::from_millis(150)); clean = remover::execute_clean(build_validated(), clean_opts()).unwrap(); continue; }
        panic!("{}", clean_diagnostics(&clean, &journal_dir));
    }
    let diag = clean_diagnostics(&clean, &journal_dir);
    assert_tier3_clean_result(&clean, &diag, &recycle, &rec_file, &rec_dir, &rust_target, &node_modules, &rust_fresh_target, &orphan_target, &node_fresh_nm, &node_orphan_nm);
}
