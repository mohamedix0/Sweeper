#![cfg(windows)]

//! Regression coverage for issue #133: the Analyzer walk must be bounded by
//! directory count during traversal (not only at tree assembly), and a
//! truncation must be reported explicitly.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use sweeper_core::analyzer::{analyze_path, AnalyzerOptions};
use sweeper_testkit::Sandbox;

fn sandbox_base_dir() -> PathBuf {
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
    base
}

fn wide_tree(child_dirs: usize) -> Sandbox {
    let sb = Sandbox::new_in(sandbox_base_dir());
    for i in 0..child_dirs {
        let dir = format!("d{i:03}");
        sb.create_dir(&dir).expect("create child directory");
        sb.write_file(format!("{dir}/payload.bin"), [0u8; 8])
            .expect("create payload file");
    }
    sb
}

fn analyze(sb: &Sandbox, options: AnalyzerOptions) -> sweeper_core::analyzer::AnalyzerResult {
    analyze_path(
        Some(&sb.root().to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        options,
    )
    .expect("analyze sandbox")
}

fn description_is_truncated(result: &sweeper_core::analyzer::AnalyzerResult) -> bool {
    result
        .root
        .description
        .to_ascii_lowercase()
        .contains("truncated")
}

#[test]
fn walk_directory_cap_stops_traversal_and_warns() {
    const CHILD_DIRS: usize = 80;
    const DIR_CAP: usize = 16;
    let sb = wide_tree(CHILD_DIRS);

    let res = analyze(
        &sb,
        AnalyzerOptions {
            worker_threads: 2,
            max_walk_dirs: DIR_CAP,
            max_child_dirs_per_dir: 64,
            ..AnalyzerOptions::default()
        },
    );

    assert!(!res.cancelled, "truncation is not cancellation");
    assert!(
        res.progress.scanned_dirs <= DIR_CAP as u64,
        "walk must respect the hard directory cap: scanned {} dirs, cap {}",
        res.progress.scanned_dirs,
        DIR_CAP
    );
    assert!(
        res.progress.scanned_dirs < (CHILD_DIRS + 1) as u64,
        "walk must stop early instead of enumerating every directory: {}",
        res.progress.scanned_dirs
    );
    assert!(
        description_is_truncated(&res),
        "truncation must be reported explicitly; root description was {:?}",
        res.root.description
    );
}

#[test]
fn walk_child_dir_cap_bounds_siblings_and_warns() {
    const CHILD_DIRS: usize = 80;
    const CHILD_CAP: usize = 8;
    let sb = wide_tree(CHILD_DIRS);

    let res = analyze(
        &sb,
        AnalyzerOptions {
            worker_threads: 2,
            max_walk_dirs: 1_000,
            max_child_dirs_per_dir: CHILD_CAP,
            ..AnalyzerOptions::default()
        },
    );

    assert!(!res.cancelled);
    assert!(
        res.progress.scanned_dirs <= (CHILD_CAP + 1) as u64,
        "children_dirs must be capped during traversal: scanned {} dirs",
        res.progress.scanned_dirs
    );
    assert!(
        description_is_truncated(&res),
        "child-dir truncation must be reported; root description was {:?}",
        res.root.description
    );
}

#[test]
fn small_tree_is_not_reported_as_truncated() {
    let sb = wide_tree(3);

    let res = analyze(
        &sb,
        AnalyzerOptions {
            worker_threads: 2,
            ..AnalyzerOptions::default()
        },
    );

    assert!(!res.cancelled);
    assert!(
        !description_is_truncated(&res),
        "an uncapped tree must not carry a truncation warning: {:?}",
        res.root.description
    );
}
