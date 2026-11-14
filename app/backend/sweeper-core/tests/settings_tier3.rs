#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sweeper_core::analyzer::{
    analyze_path, build_analyzer_preview_items, AnalyzerNode, AnalyzerOptions, AnalyzerResult,
};
use sweeper_core::remover::{self, CleanOptions, ValidatedItem};
use sweeper_core::scanner::{self, ScanSettingsOptions};
use sweeper_core::settings::{self, SweeperSettings, MAX_MIN_AGE_HOURS};
use sweeper_knowledge::Risk;
use sweeper_testkit::Sandbox;

fn sandbox_base_dir() -> PathBuf {
    fn strip_extended_prefix(p: PathBuf) -> PathBuf {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest.to_string());
        }
        p
    }

    let base = if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
        PathBuf::from(v).join("sweeper-test-sandboxes")
    } else if let Some(v) = std::env::var_os("CARGO_TARGET_TMPDIR") {
        PathBuf::from(v).join("sweeper-test-sandboxes")
    } else {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest
            .join("../../target/sweeper-test-sandboxes")
            .components()
            .collect::<PathBuf>()
    };
    let _ = fs::create_dir_all(&base);
    match std::fs::canonicalize(&base) {
        Ok(c) => strip_extended_prefix(c),
        Err(_) => base,
    }
}

fn sandbox_in_target(_name: &str) -> Sandbox {
    Sandbox::new_in(sandbox_base_dir()).unwrap()
}

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
    match fs::read_dir(journal_dir) {
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
    let Ok(entries) = fs::read_dir(journal_dir) else {
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

fn find_node_by_suffix(root: &AnalyzerNode, suffix_lower: &str) -> Option<AnalyzerNode> {
    let mut stack = vec![root.clone()];
    while let Some(n) = stack.pop() {
        if n.path.to_ascii_lowercase().ends_with(suffix_lower) {
            return Some(n.clone());
        }
        for c in &n.children {
            stack.push(c.clone());
        }
    }
    None
}

fn make_analyzer_node(
    id: &str,
    path: &str,
    risk: Risk,
    is_dir: bool,
    selectable: bool,
    display_only: bool,
) -> AnalyzerNode {
    AnalyzerNode {
        id: id.to_string(),
        path: path.to_string(),
        name: "n".to_string(),
        is_dir,
        size_bytes: 0,
        item_count: 0,
        risk,
        description: String::new(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only,
        selectable,
        selection_reason: None,
        children: vec![],
    }
}

fn wrap_analyzer_root(root: AnalyzerNode) -> AnalyzerResult {
    AnalyzerResult {
        root: root.clone(),
        progress: sweeper_core::analyzer::AnalyzerProgress::default(),
        cancelled: false,
    }
}

#[test]
fn settings_untrusted_tampered_malformed_and_protected_overrides_are_sanitized() {
    // malformed/corrupt JSON => safe defaults + warning, no panic
    let parsed = settings::parse_untrusted_settings_json("{not-json");
    assert_eq!(parsed.settings, SweeperSettings::default());
    assert!(
        !parsed.warnings.is_empty(),
        "expected sanitization warning for malformed json"
    );

    // tampered/out-of-bounds + invalid custom paths + protected overrides injected
    let custom_sb = Sandbox::new_in(sandbox_base_dir()).unwrap();
    let valid_custom = custom_sb.create_dir(r"a\b\custom_ok").unwrap();
    let missing_custom = custom_sb.root().join(r"a\b\missing_custom_path_dir");
    assert!(
        !missing_custom.exists(),
        "missing custom path fixture must not exist"
    );
    let json = format!(
        r#"{{
  "enable_tier1": false,
  "enable_tier2": "nope",
  "enable_tier3": true,
  "min_age_hours": 999999,
  "allow_delete_unknown": true,
  "custom_paths": [
    "{}",
    "{}",
    "C:\\Windows\\System32",
    "C:\\Users",
    "C:\\a\\..\\b",
    "not\\relative-or-unsafe",
    "{}"
  ],
  "protected_paths": ["C:\\Windows"],
  "exclusions": ["C:\\Windows"],
  "stale_project_days": 0
}}"#,
        valid_custom.to_string_lossy().replace('\\', "\\\\"),
        missing_custom.to_string_lossy().replace('\\', "\\\\"),
        valid_custom.to_string_lossy().replace('\\', "\\\\")
    );
    let parsed = settings::parse_untrusted_settings_json(&json);
    let s = &parsed.settings;
    assert!(!s.enable_tier1, "valid bool field should be applied");
    assert!(
        s.enable_tier2,
        "invalid bool should fall back to default true"
    );
    assert!(s.enable_tier3, "valid tier3 toggle should be applied");
    assert_eq!(
        s.min_age_hours, 24,
        "out-of-bounds min_age_hours must reset to default 24"
    );
    assert_eq!(
        s.stale_project_days, 30,
        "out-of-bounds stale_project_days must reset to default 30"
    );
    assert!(
        s.allow_delete_unknown,
        "valid allow_delete_unknown should be applied"
    );
    assert_eq!(
        s.custom_paths.len(),
        1,
        "only the valid, existing, non-duplicate custom path should remain: {:?} warnings={:?}",
        s.custom_paths,
        parsed.warnings
    );
    assert!(
        s.custom_paths[0].eq_ignore_ascii_case(&canon_str(&valid_custom)),
        "kept custom path should be canonical/validated"
    );
    assert!(
        parsed
            .warnings
            .iter()
            .any(|w| w.contains("protected") || w.contains("ignored")),
        "tampered protected/exclusion overrides must be ignored with warning: {:?}",
        parsed.warnings
    );
    assert!(
        parsed.warnings.iter().any(|w| w.contains("min_age_hours")),
        "expected min_age_hours sanitization warning: {:?}",
        parsed.warnings
    );
    assert!(
        parsed.warnings.iter().any(|w| w.contains("custom path")),
        "expected custom path sanitization warnings: {:?}",
        parsed.warnings
    );
    assert!(
        parsed
            .warnings
            .iter()
            .any(|w| w.contains("stale_project_days")),
        "expected stale_project_days sanitization warning: {:?}",
        parsed.warnings
    );
    const { assert!(MAX_MIN_AGE_HOURS >= 8760) };
}

#[test]
fn custom_path_scan_missing_yields_zero_items_and_clean_never_touches_parent() {
    let sb = Sandbox::new().unwrap();
    let parent = sb.create_dir("parent").unwrap();
    let parent_file = parent.join("keep.txt");
    write_file(&parent_file, b"keep");
    set_old(&parent_file);
    let missing = parent.join("missing_custom");
    assert!(!missing.exists());

    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        custom_paths: vec![missing.to_string_lossy().to_string()],
        ..SweeperSettings::default()
    };
    let report =
        scanner::scan_with_settings_options(&settings, ScanSettingsOptions::default()).unwrap();
    let cp = report
        .targets
        .iter()
        .find(|t| t.target_id.starts_with("custom-path"))
        .expect("custom path target result present");
    assert!(
        cp.items.is_empty() && cp.total_bytes == 0 && cp.file_count == 0,
        "missing custom path must yield zero items: {:?}",
        cp
    );
    assert!(
        cp.status_reason
            .as_deref()
            .unwrap_or("")
            .contains("no fallback")
            || !cp.skipped_diagnostics.is_empty()
            || !cp.scanned
            || cp.items.is_empty(),
        "missing custom path should be reported without fallback: {:?}",
        cp
    );

    // Even if a caller tried to construct a validated item for a missing root, remover must not touch parent.
    let lex =
        sweeper_core::paths::CanonicalPath::parse_lexical(&missing.to_string_lossy()).unwrap();
    let item = sweeper_core::remover::validated_item_for_tests(
        "missing-custom",
        "custom-path",
        lex.as_str().to_string(),
        lex.as_str().to_string(),
        0,
        Some(old_mtime_secs()),
        sweeper_core::remover::ValidatedSource::CustomPath,
    );
    let journal_dir = sb.create_dir("journal").unwrap();
    let clean = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 0,
            min_age: Duration::from_secs(0),
            now: SystemTime::now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();
    assert!(parent.exists(), "parent directory must remain");
    assert!(
        parent_file.exists(),
        "parent contents must remain untouched"
    );
    assert_eq!(
        clean.bytes_freed, 0,
        "missing custom path must produce zero deletions"
    );
}

#[test]
fn custom_path_scan_and_clean_contents_only_preserves_root() {
    let sb = sandbox_in_target("custom-path-scan-clean");
    let custom = sb.create_dir(r"a\b\custom_root").unwrap();
    let f1 = custom.join("a.txt");
    let sub = custom.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let f2 = sub.join("b.txt");
    write_file(&f1, b"aaa");
    write_file(&f2, b"bbbbb");
    set_old(&f1);
    set_old(&sub);
    set_old(&f2);

    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        custom_paths: vec![custom.to_string_lossy().to_string()],
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let report =
        scanner::scan_with_settings_options(&settings, ScanSettingsOptions::default()).unwrap();
    let cp = report
        .targets
        .iter()
        .find(|t| t.target_id.starts_with("custom-path"))
        .expect("custom path target result");
    assert!(
        cp.scanned,
        "custom path should scan: {:?}",
        cp.status_reason
    );
    assert!(
        cp.items
            .iter()
            .all(|i| !canon_str(&custom).eq_ignore_ascii_case(
                &sweeper_core::paths::CanonicalPath::parse_lexical(&i.path)
                    .unwrap()
                    .to_string()
            )),
        "scan must not include custom path root directory itself as a removable entry: {:?}",
        cp.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    assert!(
        cp.file_count >= 2,
        "expected scanned child files: {:?}",
        cp.items
    );

    // Clean contents only via custom-path root item (preserves root directory).
    let root_item =
        ValidatedItem::from_custom_path_root("custom-root", canon_str(&custom), cp.total_bytes)
            .unwrap();
    let journal_dir = sb.create_dir("journal-custom").unwrap();
    let clean = remover::execute_clean(
        vec![root_item],
        CleanOptions {
            preview_estimated_bytes: cp.total_bytes.max(1),
            min_age: Duration::from_secs(0),
            now: SystemTime::now(),
            journal_base_dir: Some(journal_dir.clone()),
        },
    )
    .unwrap();
    let diag = clean_diagnostics(&clean, &journal_dir);
    assert!(
        !clean.aborted,
        "clean aborted: {:?}\n{}",
        clean.abort_reason, diag
    );
    assert!(
        custom.exists(),
        "custom path root directory must be preserved\n{}",
        diag
    );
    assert!(!f1.exists(), "child file should be cleaned\n{}", diag);
    assert!(
        !f2.exists(),
        "nested child file should be cleaned\n{}",
        diag
    );
    // subdirectory may be removed as empty child; root must remain.
}

#[test]
fn tier3_disabled_by_default_yields_no_tier3_conditional_items() {
    let sb = Sandbox::new().unwrap();
    let search_root = sb.create_dir("projects").unwrap();
    // Create would-be stale artifacts; Tier 3 disabled so they must not be discovered/reported.
    let proj = search_root.join("proj");
    let target = proj.join("target");
    fs::create_dir_all(&target).unwrap();
    write_file(
        &proj.join("Cargo.toml"),
        b"[package]\nname='x'\nversion='0.0.0'\n",
    );
    write_file(&target.join("old.bin"), b"abc");
    let stale_time =
        sweeper_testkit::filetime_from_unix_secs(old_mtime_secs().saturating_sub(40 * 24 * 3600));
    sweeper_testkit::set_last_write_time(&proj.join("Cargo.toml"), stale_time).unwrap();
    set_old(&target.join("old.bin"));

    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: false,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: vec![search_root.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();
    assert!(
        report.targets.iter().all(|t| t.target_id != "recycle-bin"
            && t.target_id != "stale-rust-target"
            && t.target_id != "stale-node-modules"),
        "tier 3 conditional targets must not run when disabled: {:?}",
        report
            .targets
            .iter()
            .map(|t| &t.target_id)
            .collect::<Vec<_>>()
    );
}

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

#[test]
fn tier3_stale_rust_target_discovery_respects_marker_and_age() {
    let sb = Sandbox::new().unwrap();
    let (search_root, rust_target, rust_fresh_target, orphan_target, _, _, _, recycle, _, _) =
        setup_tier3_scan_fixture(&sb);
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        stale_project_days: 30,
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: vec![search_root.to_string_lossy().to_string()],
        recycle_bin_roots: vec![recycle.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();
    let stale_rust = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-rust-target")
        .expect("stale rust target result");
    assert!(
        stale_rust.scanned,
        "stale rust target should be scanned: {:?}",
        stale_rust.status_reason
    );
    assert_eq!(
        stale_rust.items.len(),
        1,
        "only stale rust target dir should be included: {:?}",
        stale_rust.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    assert!(stale_rust.items[0]
        .path
        .to_ascii_lowercase()
        .ends_with("\\target"));
    assert!(rust_fresh_target.exists(), "fresh rust target must remain");
    assert!(
        orphan_target.exists(),
        "target without sibling Cargo.toml must remain"
    );
    assert!(rust_target.exists(), "fixture precondition");
}

#[test]
fn tier3_stale_node_modules_discovery_respects_lockfiles_and_age() {
    let sb = Sandbox::new().unwrap();
    let (search_root, _, _, _, node_modules, node_fresh_nm, node_orphan_nm, recycle, _, _) =
        setup_tier3_scan_fixture(&sb);
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        stale_project_days: 30,
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: vec![search_root.to_string_lossy().to_string()],
        recycle_bin_roots: vec![recycle.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();
    let stale_nm = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-node-modules")
        .expect("stale node_modules target result");
    assert!(
        stale_nm.scanned,
        "stale node_modules should be scanned: {:?}",
        stale_nm.status_reason
    );
    assert_eq!(
        stale_nm.items.len(),
        1,
        "only stale node_modules dir should be included: {:?}",
        stale_nm.items.iter().map(|i| &i.path).collect::<Vec<_>>()
    );
    assert!(stale_nm.items[0]
        .path
        .to_ascii_lowercase()
        .ends_with("\\node_modules"));
    assert!(
        node_fresh_nm.exists(),
        "node_modules with fresh lockfile must remain"
    );
    assert!(
        node_orphan_nm.exists(),
        "node_modules without lockfile must remain"
    );
    assert!(node_modules.exists(), "fixture precondition");
}

#[test]
fn tier3_recycle_bin_scan_reports_contents_in_sandbox() {
    let sb = Sandbox::new().unwrap();
    let (search_root, _, _, _, _, _, _, recycle, _, _) = setup_tier3_scan_fixture(&sb);
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        stale_project_days: 30,
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: vec![search_root.to_string_lossy().to_string()],
        recycle_bin_roots: vec![recycle.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();
    let rb = report
        .targets
        .iter()
        .find(|t| t.target_id == "recycle-bin")
        .expect("recycle-bin target result");
    assert!(
        rb.scanned,
        "sandbox recycle-bin should scan: {:?}",
        rb.status_reason
    );
    assert!(
        rb.file_count >= 2,
        "recycle-bin contents should be counted: {:?}",
        rb
    );
}

#[test]
fn tier3_clean_removes_stale_dirs_and_recycle_contents_only() {
    let sb = Sandbox::new().unwrap();
    let (
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
    ) = setup_tier3_scan_fixture(&sb);
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        stale_project_days: 30,
        min_age_hours: 0,
        ..SweeperSettings::default()
    };
    let opts = ScanSettingsOptions {
        tier3_project_search_roots: vec![search_root.to_string_lossy().to_string()],
        recycle_bin_roots: vec![recycle.to_string_lossy().to_string()],
        ..Default::default()
    };
    let report = scanner::scan_with_settings_options(&settings, opts).unwrap();
    let stale_rust = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-rust-target")
        .unwrap();
    let stale_nm = report
        .targets
        .iter()
        .find(|t| t.target_id == "stale-node-modules")
        .unwrap();
    let rb = report
        .targets
        .iter()
        .find(|t| t.target_id == "recycle-bin")
        .unwrap();

    let build_validated = || {
        vec![
            ValidatedItem::from_tier3_recycle_bin_root("rb", canon_str(&recycle), rb.total_bytes)
                .unwrap(),
            ValidatedItem::from_tier3_project_directory(
                "rust-target",
                "stale-rust-target",
                canon_str(&rust_target),
                stale_rust.total_bytes,
                Some(old_mtime_secs()),
            )
            .unwrap(),
            ValidatedItem::from_tier3_project_directory(
                "node-modules",
                "stale-node-modules",
                canon_str(&node_modules),
                stale_nm.total_bytes,
                Some(old_mtime_secs()),
            )
            .unwrap(),
        ]
    };
    let journal_dir = sb.create_dir("journal").unwrap();
    let clean_opts = || CleanOptions {
        preview_estimated_bytes: rb.total_bytes
            + stale_rust.total_bytes
            + stale_nm.total_bytes
            + 1024,
        min_age: Duration::from_secs(0),
        now: SystemTime::now(),
        journal_base_dir: Some(journal_dir.clone()),
    };

    let mut clean = remover::execute_clean(build_validated(), clean_opts()).unwrap();
    for attempt in 0..3 {
        let targets_removed = !rec_file.exists()
            && !rec_dir.exists()
            && !rust_target.exists()
            && !node_modules.exists();
        if targets_removed {
            break;
        }
        let locked = journal_has_locked_skip(&journal_dir);
        if attempt < 2 && locked {
            std::thread::sleep(Duration::from_millis(150));
            clean = remover::execute_clean(build_validated(), clean_opts()).unwrap();
            continue;
        }
        let diag = clean_diagnostics(&clean, &journal_dir);
        panic!(
            "tier3 clean left targets behind after attempt {} (locked_skip={}): rec_file_exists={} rec_dir_exists={} rust_target_exists={} node_modules_exists={}\n{}",
            attempt + 1,
            locked,
            rec_file.exists(),
            rec_dir.exists(),
            rust_target.exists(),
            node_modules.exists(),
            diag
        );
    }

    let diag = clean_diagnostics(&clean, &journal_dir);
    assert!(
        !clean.aborted,
        "tier 3 clean aborted: {:?}\n{}",
        clean.abort_reason, diag
    );
    assert!(
        recycle.exists(),
        "recycle-bin root must be preserved\n{}",
        diag
    );
    assert!(
        !rec_file.exists() && !rec_dir.exists(),
        "recycle-bin contents should be removed\n{}",
        diag
    );
    assert!(
        !rust_target.exists(),
        "stale rust target directory should be removed\n{}",
        diag
    );
    assert!(
        !node_modules.exists(),
        "stale node_modules directory should be removed\n{}",
        diag
    );
    assert!(
        rust_fresh_target.exists(),
        "fresh rust target must remain\n{}",
        diag
    );
    assert!(
        orphan_target.exists(),
        "target without sibling Cargo.toml must remain\n{}",
        diag
    );
    assert!(
        node_fresh_nm.exists(),
        "node_modules with fresh lockfile must remain\n{}",
        diag
    );
    assert!(
        node_orphan_nm.exists(),
        "node_modules without lockfile must remain\n{}",
        diag
    );
}

#[test]
fn analyzer_allow_unknown_selectable_dir_requires_typed_confirmation_and_cleans_contents_only() {
    let sb = Sandbox::new().unwrap();
    let unknown_ok = sb.create_dir("unknown_ok").unwrap();
    write_file(&unknown_ok.join("f.txt"), b"x");
    set_old(&unknown_ok.join("f.txt"));
    let res = analyze_path(
        Some(&sb.root().to_string_lossy()),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 1,
            allow_unknown: true,
            ..AnalyzerOptions::default()
        },
    )
    .unwrap();
    let node = find_node_by_suffix(&res.root, "\\unknown_ok").expect("unknown_ok node");
    if node.risk == Risk::Unknown {
        assert!(node.is_dir);
        assert!(
            node.selectable,
            "eligible unknown dir should be selectable: {:?}",
            node.selection_reason
        );
        let preview = build_analyzer_preview_items(&res, std::slice::from_ref(&node.id)).unwrap();
        assert_eq!(preview.len(), 1);
        assert!(
            preview[0].requires_typed_confirmation,
            "unknown selection must require typed confirmation"
        );
        assert_eq!(preview[0].group_id, "custom-path");
        let journal_dir = sb.create_dir("journal").unwrap();
        let clean = remover::execute_clean(
            vec![preview[0].validated_item.clone()],
            CleanOptions {
                preview_estimated_bytes: preview[0].size_bytes.max(1),
                min_age: Duration::from_secs(0),
                now: SystemTime::now(),
                journal_base_dir: Some(journal_dir.clone()),
            },
        )
        .unwrap();
        let diag = clean_diagnostics(&clean, &journal_dir);
        assert!(
            !clean.aborted,
            "clean aborted: {:?}\n{}",
            clean.abort_reason, diag
        );
        assert!(
            unknown_ok.exists(),
            "unknown custom-path root must be preserved (contents-only)\n{}",
            diag
        );
        assert!(
            !unknown_ok.join("f.txt").exists(),
            "contents should be cleaned\n{}",
            diag
        );
    } else {
        assert_ne!(node.risk, Risk::Protected);
    }
}

#[test]
fn analyzer_rejects_ineligible_unknown_shallow_path() {
    let shallow = make_analyzer_node("s", r"C:\Temp", Risk::Unknown, true, true, false);
    assert!(build_analyzer_preview_items(
        &wrap_analyzer_root(shallow.clone()),
        std::slice::from_ref(&shallow.id)
    )
    .is_err());
}

#[test]
fn analyzer_rejects_ineligible_unknown_ancestor_of_protected() {
    let ancestor_protected = make_analyzer_node("a", r"C:\Users", Risk::Unknown, true, true, false);
    assert!(build_analyzer_preview_items(
        &wrap_analyzer_root(ancestor_protected.clone()),
        std::slice::from_ref(&ancestor_protected.id)
    )
    .is_err());
}

#[test]
fn analyzer_rejects_ineligible_unknown_vhdx_and_protected_link_cases() {
    let vhdx = make_analyzer_node(
        "v",
        r"C:\Users\Public\vm.vhdx",
        Risk::Unknown,
        true,
        true,
        false,
    );
    assert!(build_analyzer_preview_items(
        &wrap_analyzer_root(vhdx.clone()),
        std::slice::from_ref(&vhdx.id)
    )
    .is_err());

    let mut protected = make_analyzer_node(
        "p",
        r"C:\Windows\System32",
        Risk::Protected,
        true,
        true,
        false,
    );
    protected.selectable = false;
    assert!(build_analyzer_preview_items(
        &wrap_analyzer_root(protected.clone()),
        &[protected.id.clone()]
    )
    .is_err());

    let mut link = make_analyzer_node(
        "l",
        r"C:\Temp\SomeLinkDir",
        Risk::Unknown,
        true,
        true,
        false,
    );
    link.is_link_point = true;
    assert!(
        build_analyzer_preview_items(&wrap_analyzer_root(link.clone()), &[link.id.clone()])
            .is_err()
    );
}
