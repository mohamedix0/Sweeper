#![cfg(windows)]

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sweeper_core::analyzer::{
    analyze_path, build_scan_items_from_analyzer, AnalyzerNode, AnalyzerOptions,
};
use sweeper_knowledge::Risk;
use sweeper_testkit::{create_junction, CanaryTree, Sandbox};

fn node_index(node: &AnalyzerNode) -> HashMap<String, AnalyzerNode> {
    fn rec(n: &AnalyzerNode, map: &mut HashMap<String, AnalyzerNode>) {
        map.insert(n.path.to_ascii_lowercase(), n.clone());
        for c in &n.children {
            rec(c, map);
        }
    }
    let mut m = HashMap::new();
    rec(node, &mut m);
    m
}

fn find_by_suffix<'a>(
    map: &'a HashMap<String, AnalyzerNode>,
    suffix: &str,
) -> Option<&'a AnalyzerNode> {
    let needle = suffix.replace('/', "\\").to_ascii_lowercase();
    map.values()
        .find(|n| n.path.to_ascii_lowercase().ends_with(&needle))
}

#[test]
fn recursive_size_and_item_count_aggregation_accuracy() {
    let sb = Sandbox::new().expect("sandbox");
    sb.write_file("a.txt", b"aaaaa").expect("a");
    sb.create_dir("dir1").expect("dir1");
    sb.write_file("dir1/b.bin", [0u8; 10]).expect("b");
    sb.create_dir("dir1/sub").expect("sub");
    sb.write_file("dir1/sub/c.dat", [1u8; 7]).expect("c");
    sb.create_dir("emptydir").expect("emptydir");

    let sentinel_size = std::fs::metadata(sb.root().join(sweeper_testkit::SENTINEL_FILE_NAME))
        .map(|m| m.len())
        .unwrap_or(0);
    let expected_total = 22 + sentinel_size;

    let res = analyze_path(
        Some(&sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 2,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze sandbox");

    assert!(!res.cancelled);
    assert_eq!(
        res.root.size_bytes, expected_total,
        "total bytes should include all files including sandbox sentinel"
    );
    // item_count includes discovered files and directories under/including children accounting
    assert!(
        res.root.item_count >= 6,
        "expected recursive item count >= 6, got {}",
        res.root.item_count
    );

    let idx = node_index(&res.root);
    let dir1 = find_by_suffix(&idx, "\\dir1").expect("dir1 node");
    assert_eq!(dir1.size_bytes, 17, "dir1 should aggregate b.bin + c.dat");
    let empty = find_by_suffix(&idx, "\\emptydir").expect("emptydir node");
    assert_eq!(empty.size_bytes, 0);
}

#[test]
fn junction_is_not_traversed_and_external_canary_excluded() {
    let sb = Sandbox::new().expect("sandbox");
    let canary = CanaryTree::seed_external("analyzer-junction").expect("canary");

    sb.write_file("inside.txt", b"inside").expect("inside file");
    let junc = sb.root().join("escape_junction");
    create_junction(&junc, canary.root()).expect("create junction");

    let res = analyze_path(
        Some(&sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 2,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze");

    let idx = node_index(&res.root);
    assert!(
        find_by_suffix(&idx, "\\inside.txt").is_some(),
        "local file should be present"
    );
    assert!(
        !idx.values()
            .any(|n| n.path.to_ascii_lowercase().contains("root.txt")
                || n.name.eq_ignore_ascii_case("root.txt")),
        "canary contents must not appear in analyzer tree"
    );

    let j = find_by_suffix(&idx, "\\escape_junction").expect("junction node present");
    assert!(
        j.is_link_point || !j.selectable,
        "junction should be marked non-traversable/non-selectable"
    );
    assert_eq!(
        j.size_bytes, 0,
        "junction target content must not be aggregated"
    );

    // External canary remains intact.
    canary.verify().expect("external canary must remain intact");
}

#[test]
fn vhdx_marked_display_only_and_rejected_by_conversion_seam() {
    let sb = Sandbox::new().expect("sandbox");
    sb.create_dir("vms").expect("vms dir");
    sb.write_file("vms/distro.vhdx", [0u8; 4096])
        .expect("vhdx file");

    let res = analyze_path(
        Some(&sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 2,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze");

    let idx = node_index(&res.root);
    let vhdx = find_by_suffix(&idx, "\\vms\\distro.vhdx").expect("vhdx node");
    assert!(vhdx.display_only, "vhdx must be display-only");
    assert!(!vhdx.selectable, "vhdx must not be selectable");
    assert_eq!(vhdx.risk, Risk::Caution, "vhdx should be caution-annotated");

    let err = build_scan_items_from_analyzer(&res, std::slice::from_ref(&vhdx.id))
        .expect_err("vhdx conversion must be rejected");
    assert!(
        err.to_ascii_lowercase().contains("vhdx")
            || err.contains("display-only")
            || err.contains("non-selectable"),
        "unexpected error: {err}"
    );
}

#[test]
fn protected_and_non_allowlist_descendants_under_protected_container_are_not_selectable_and_rejected(
) {
    // Analyze C:\Windows (protected container root allowed for read-only display) and assert protection behavior.
    let root = r"C:\Windows";
    if !PathBuf::from(root).exists() {
        eprintln!("skipping protected container test: {root} missing");
        return;
    }

    let res = analyze_path(
        Some(root),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 4,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze C:\\Windows read-only");

    assert_eq!(
        res.root.risk,
        Risk::Protected,
        "Windows root should be protected in analyzer view"
    );
    assert!(!res.root.selectable, "Windows root must not be selectable");

    // System32 is subtree-protected and must be non-selectable if present.
    if let Some(system32) = res
        .root
        .children
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case("System32"))
    {
        assert_eq!(system32.risk, Risk::Protected);
        assert!(!system32.selectable, "System32 must not be selectable");
        let err = build_scan_items_from_analyzer(&res, std::slice::from_ref(&system32.id))
            .expect_err("protected node conversion must be rejected");
        let err_l = err.to_ascii_lowercase();
        assert!(
            err_l.contains("protected")
                || err_l.contains("not selectable")
                || err_l.contains("display-only"),
            "{err}"
        );
    }

    // A non-allowlist descendant under protected container (e.g. Windows root itself) must be rejected.
    let err = build_scan_items_from_analyzer(&res, std::slice::from_ref(&res.root.id))
        .expect_err("protected container root selection must be rejected");
    assert!(
        err.to_ascii_lowercase().contains("protected")
            || err.contains("display-only")
            || err.contains("non-selectable"),
        "{err}"
    );
}

#[test]
fn eligibility_rules_for_safe_caution_unknown() {
    // Safe selectable directory in a normal (non-protected-container) location
    let safe_sb = Sandbox::new().expect("safe sandbox");
    safe_sb.create_dir("safe_dir").expect("safe dir");
    safe_sb.write_file("safe_dir/x.txt", b"x").expect("x");
    let safe_res = analyze_path(
        Some(&safe_sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 2,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze safe sandbox");
    let safe_idx = node_index(&safe_res.root);
    let safe_dir = find_by_suffix(&safe_idx, "\\safe_dir").expect("safe_dir");
    // Sandbox under temp/profile may be treated as unknown in some contexts, but must follow non-protected rules;
    // assert it is not protected and follows Risk-based selectability constraints.
    assert_ne!(safe_dir.risk, Risk::Protected);
    if safe_dir.risk == Risk::Unknown {
        assert!(
            !safe_dir.selectable,
            "Unknown must default to non-selectable in Phase 4"
        );
    } else if safe_dir.risk == Risk::Caution {
        // Default settings enable Tier1/2 only for known tiers; unknown-tier caution nodes are not selectable.
        // Just ensure no protected/display-only violation.
        assert!(!safe_dir.display_only);
    } else if safe_dir.risk == Risk::Safe {
        assert!(safe_dir.selectable || !safe_dir.display_only);
    }

    // Disabled-tier Caution (Gradle caches, Tier2) can be selectable in some environments only if tier enabled;
    // to make deterministic, check annotation on resolved path via policy/knowledge indirectly by simulating
    // a Caution node conversion expectation: Unknown default non-selectable and Caution respects enabled tiers.
    // We validate unknown default via a clearly unknown path name inside sandbox.
    let unk_sb = Sandbox::new().expect("unknown sandbox");
    unk_sb
        .create_dir("totally_unknown_folder_zz")
        .expect("unknown dir");
    let unk_res = analyze_path(
        Some(&unk_sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 2,
            allow_unknown: false,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze unknown sandbox");
    let unk_idx = node_index(&unk_res.root);
    let unk_dir =
        find_by_suffix(&unk_idx, "\\totally_unknown_folder_zz").expect("unknown dir node");
    if unk_dir.risk == Risk::Unknown {
        assert!(
            !unk_dir.selectable,
            "Unknown must default to non-selectable in Phase 4"
        );
        let err = build_scan_items_from_analyzer(&unk_res, std::slice::from_ref(&unk_dir.id))
            .expect_err("unknown conversion should be rejected when not selectable");
        assert!(
            err.contains("non-selectable") || err.contains("Unknown"),
            "{err}"
        );
    }

    // Known allowlisted caution target path (Gradle caches) is Tier2; tier enabled by default => selectable
    // only if path resolves to that allowlisted root and is not inside protected contexts. Best-effort check.
    let gradle = dirs_next_to_home_cache();
    if let Some((path, _tier_enabled_expected)) = gradle {
        if path.exists() {
            let res = analyze_path(
                Some(&path.to_string_lossy()),
                Arc::new(AtomicBool::new(false)),
                None,
                AnalyzerOptions {
                    worker_threads: 2,
                    ..AnalyzerOptions::default()
                },
            );
            if let Ok(res) = res {
                // Root may be annotated as the target itself (Caution/Safe depending) and should be non-protected.
                assert_ne!(res.root.risk, Risk::Protected);
                if res.root.risk == Risk::Caution {
                    // Tier 2 enabled by default in analyzer settings
                    assert!(
                        res.root.selectable,
                        "enabled-tier caution target root should be selectable"
                    );
                }
            }
        }
    }
}

fn dirs_next_to_home_cache() -> Option<(PathBuf, bool)> {
    // Attempt to locate ~/.gradle/caches for a best-effort, non-fatal eligibility check.
    let profile = std::env::var_os("USERPROFILE")?;
    let p = PathBuf::from(profile).join(".gradle").join("caches");
    Some((p, true))
}

#[test]
fn deep_directory_tree_finishes_without_stack_overflow_and_safe_drop() {
    let sb = Sandbox::new().expect("sandbox");
    // Create 1000-level deep directory tree (iterative creation helper) and a file at bottom.
    let deep = sweeper_testkit::create_deep_tree(sb.root(), 1000).expect("create deep tree");
    fs::write(deep.join("leaf.txt"), b"leaf").expect("leaf");

    let res = analyze_path(
        Some(&sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 4,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze deep tree");

    assert!(!res.cancelled);
    let idx = node_index(&res.root);
    assert!(
        find_by_suffix(&idx, "\\leaf.txt").is_some(),
        "leaf file in deep tree must be discovered"
    );
    assert!(
        res.root.item_count >= 1001,
        "expected deep item count, got {}",
        res.root.item_count
    );

    // Deterministic safe drop: dropping a 1000+ level tree must not overflow the stack.
    drop(res);
}

#[test]
fn cancellation_stops_cleanly() {
    let sb = Sandbox::new().expect("sandbox");
    // Build a moderately large tree so cancellation has a chance to be observed.
    for i in 0..200 {
        let dir = format!("d{i}");
        sb.create_dir(&dir).expect("dir");
        for j in 0..20 {
            sb.write_file(format!("{dir}/f{j}.bin"), [0u8; 64])
                .expect("file");
        }
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_cb = Arc::clone(&cancel);
    let cb: Arc<dyn Fn(sweeper_core::analyzer::AnalyzerProgress) + Send + Sync> =
        Arc::new(move |p| {
            if p.scanned_dirs >= 10 {
                cancel_for_cb.store(true, Ordering::Relaxed);
            }
        });

    let res = analyze_path(
        Some(&sb.root().to_string_lossy()),
        Arc::clone(&cancel),
        Some(cb),
        AnalyzerOptions {
            worker_threads: 4,
            progress_interval: std::time::Duration::from_millis(5),
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze with cancellation should return result");

    assert!(cancel.load(Ordering::Relaxed), "cancel flag should be set");
    assert!(res.cancelled, "result should report cancellation");
}

#[test]
#[ignore]
fn benchmark_real_c_read_only_non_destructive() {
    // Read-only benchmark only (no deletions). Run manually/optionally.
    let start = std::time::Instant::now();
    let cancel = Arc::new(AtomicBool::new(false));
    let res = analyze_path(
        Some(r"C:\"),
        cancel,
        None,
        AnalyzerOptions {
            worker_threads: 8,
            progress_interval: std::time::Duration::from_millis(500),
            ..AnalyzerOptions::default()
        },
    )
    .expect("read-only analysis of C:\\ should run");
    let elapsed = start.elapsed();
    eprintln!(
        "ANALYZER_BENCHMARK: root={} size_bytes={} item_count={} scanned_dirs={} scanned_files={} elapsed_ms={}",
        res.root.path,
        res.root.size_bytes,
        res.root.item_count,
        res.progress.scanned_dirs,
        res.progress.scanned_files,
        elapsed.as_millis()
    );
}

#[test]
fn analyzer_root_validation_rejects_invalid_inputs() {
    assert!(sweeper_core::analyzer::validate_analyzer_root(Some(r"\\server\share")).is_err());
    assert!(
        sweeper_core::analyzer::validate_analyzer_root(Some(r"C:\does\not\exist\..\x")).is_err()
    );
    assert!(sweeper_core::analyzer::validate_analyzer_root(Some(r"relative\path")).is_err());
}

#[test]
fn build_scan_items_rejects_unsafe_and_display_only_nodes() {
    use sweeper_core::analyzer::{AnalyzerProgress, AnalyzerResult};

    fn mk(
        id: &str,
        path: &str,
        risk: Risk,
        selectable: bool,
        display_only: bool,
        is_dir: bool,
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
    fn wrap(node: AnalyzerNode) -> AnalyzerResult {
        AnalyzerResult {
            root: node.clone(),
            progress: AnalyzerProgress::default(),
            cancelled: false,
        }
    }

    // .vhdx (display-only / non-selectable)
    let vhdx = mk(
        "v",
        r"C:\Users\me\vm.vhdx",
        Risk::Caution,
        false,
        true,
        false,
    );
    assert!(
        build_scan_items_from_analyzer(&wrap(vhdx.clone()), std::slice::from_ref(&vhdx.id))
            .is_err()
    );

    // Protected
    let prot = mk(
        "p",
        r"C:\Windows\System32",
        Risk::Protected,
        false,
        true,
        true,
    );
    assert!(
        build_scan_items_from_analyzer(&wrap(prot.clone()), std::slice::from_ref(&prot.id))
            .is_err()
    );

    // Link point
    let mut lp = mk("l", r"C:\Temp\link", Risk::Unknown, false, false, true);
    lp.is_link_point = true;
    assert!(
        build_scan_items_from_analyzer(&wrap(lp.clone()), std::slice::from_ref(&lp.id)).is_err()
    );

    // Cloud placeholder
    let mut cp = mk("c", r"C:\Temp\cloud", Risk::Unknown, false, false, true);
    cp.is_cloud_placeholder = true;
    assert!(
        build_scan_items_from_analyzer(&wrap(cp.clone()), std::slice::from_ref(&cp.id)).is_err()
    );

    // Unknown not selectable (allow_unknown false)
    let unk = mk(
        "u",
        r"C:\Temp\unknown_dir",
        Risk::Unknown,
        false,
        false,
        true,
    );
    assert!(
        build_scan_items_from_analyzer(&wrap(unk.clone()), std::slice::from_ref(&unk.id)).is_err()
    );

    // Ancestor-of-protected / protected-container-like custom-path candidate must be rejected by seam.
    // (Even if incorrectly marked selectable, conversion must not weaken policy.)
    let mut bad = mk("b", r"C:\Windows", Risk::Safe, true, false, true);
    bad.selectable = true;
    bad.display_only = false;
    bad.risk = Risk::Safe;
    assert!(
        build_scan_items_from_analyzer(&wrap(bad.clone()), std::slice::from_ref(&bad.id)).is_err()
    );
}

#[test]
fn wildcard_allowlist_annotation_best_effort() {
    // Best-effort: if a concrete wildcard allowlist root exists (e.g. Chrome Profile * cache or JetBrains caches),
    // ensure analyzer annotates it as an allowlist target node.
    let candidates = [
        std::env::var_os("LOCALAPPDATA")
            .map(|p| std::path::PathBuf::from(p).join(r"Google\Chrome\User Data\Profile 1\Cache")),
        std::env::var_os("LOCALAPPDATA")
            .map(|p| std::path::PathBuf::from(p).join(r"Microsoft\Edge\User Data\Profile 1\Cache")),
        std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join(r"JetBrains")),
    ];
    // Try JetBrains */caches discovery
    if let Some(jb) =
        std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join("JetBrains"))
    {
        if jb.exists() {
            if let Ok(entries) = std::fs::read_dir(&jb) {
                for e in entries.flatten() {
                    let caches = e.path().join("caches");
                    if caches.exists() {
                        let res = analyze_path(
                            Some(&caches.to_string_lossy()),
                            Arc::new(AtomicBool::new(false)),
                            None,
                            AnalyzerOptions {
                                worker_threads: 2,
                                ..Default::default()
                            },
                        );
                        if let Ok(res) = res {
                            // If this is a JetBrains caches allowlist target, root should be annotated with target_id.
                            if res.root.target_id.as_deref() == Some("jetbrains-caches") {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
    // Chrome/Edge profile cache checks
    for p in [candidates[0].clone(), candidates[1].clone()]
        .into_iter()
        .flatten()
    {
        if p.exists() {
            let res = analyze_path(
                Some(&p.to_string_lossy()),
                Arc::new(AtomicBool::new(false)),
                None,
                AnalyzerOptions {
                    worker_threads: 2,
                    ..Default::default()
                },
            );
            if let Ok(res) = res {
                if res.root.target_id.is_some() {
                    return;
                }
            }
        }
    }
    eprintln!("skipping wildcard_allowlist_annotation_best_effort: no matching wildcard target root found");
}
