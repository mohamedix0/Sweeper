#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only fixture setup; production removal is enforced in core::remover.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sweeper_core::analyzer::{analyze_path, AnalyzerNode, AnalyzerOptions, AnalyzerResult};
use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, ValidatedSource};
use sweeper_testkit::{CanaryTree, Sandbox};

mod common;
use common::*;

/// Number of display entries kept per directory in the Analyzer tests.
const CAP: usize = 8;

fn make_many_files(root: &Path, count: usize) -> u64 {
    let mut expected_bytes = 0u64;
    for i in 0..count {
        // Unique, monotonically increasing sizes so the top-N ordering is unambiguous.
        let size = (i + 1) as u64;
        let name = format!("f{i:05}.bin");
        let path = root.join(name);
        write_file(&path, &vec![b'x'; size as usize]);
        sweeper_testkit::set_last_write_time(
            &path,
            sweeper_testkit::filetime_from_unix_secs(old_mtime_secs()),
        )
        .unwrap();
        expected_bytes += size;
    }
    expected_bytes
}

fn find_node_by_suffix<'a>(node: &'a AnalyzerNode, suffix: &str) -> Option<&'a AnalyzerNode> {
    if node.path.ends_with(suffix) {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_node_by_suffix(child, suffix))
}

#[test]
fn analyzer_prunes_many_files_with_bounded_children_and_exact_totals() {
    let sb = Sandbox::new();
    let dir = sb.create_dir("huge").unwrap();
    const FILES: usize = 6000;
    let expected_bytes = make_many_files(&dir, FILES);

    let start = Instant::now();
    let res = analyze_path(
        Some(&dir.to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 4,
            max_files_per_dir: CAP,
            max_children_per_dir: CAP,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze many-file sandbox");
    let elapsed = start.elapsed();
    eprintln!(
        "ISSUE97_ANALYZER: files={} elapsed_ms={} retained_children={}",
        res.progress.scanned_files,
        elapsed.as_millis(),
        res.root.children.len()
    );

    assert!(
        elapsed < Duration::from_secs(30),
        "analyzer exceeded time budget: {elapsed:?}"
    );
    assert_eq!(
        res.progress.scanned_files, FILES as u64,
        "all files must be scanned"
    );
    assert_eq!(
        res.progress.scanned_bytes, expected_bytes,
        "progress bytes must remain exact"
    );
    assert_eq!(
        res.root.size_bytes, expected_bytes,
        "recursive size must remain exact"
    );

    // Bounded pruning: only CAP display entries survive even though FILES files exist.
    assert!(
        res.root.children.len() <= CAP,
        "pruning must retain at most {CAP} children, got {}",
        res.root.children.len()
    );
    let top_sizes: Vec<u64> = res.root.children.iter().map(|c| c.size_bytes).collect();
    let expected_top: Vec<u64> = (0..FILES).map(|i| (i + 1) as u64).rev().take(CAP).collect();
    assert_eq!(
        top_sizes, expected_top,
        "retained children must be the largest files by size"
    );
    assert_sentinel_intact(&sb);
}

const MULTI_DIRS: usize = 20;
const MULTI_SUBDIRS: usize = 12;
const MULTI_FILES: usize = 16;

/// Exact totals for the seeded multi-level tree (root + dirs + leaf dirs).
struct MultiTreeTotals {
    files: u64,
    bytes: u64,
    dirs: u64,
}

/// Seed one leaf directory with consecutive unique sizes; return its byte sum.
fn seed_multi_leaf_dir(dir: &Path, first_size: u64) -> u64 {
    let mut total = 0u64;
    for f in 0..MULTI_FILES {
        let size = first_size + f as u64;
        let path = dir.join(format!("f{f:02}.bin"));
        write_file(&path, &vec![b'x'; size as usize]);
        set_filetime(&path, old_mtime_secs());
        total += size;
    }
    total
}

fn seed_multi_level_tree(root: &Path) -> MultiTreeTotals {
    let mut totals = MultiTreeTotals {
        files: 0,
        bytes: 0,
        dirs: 1,
    };
    let mut next_size = 1u64;
    for d in 0..MULTI_DIRS {
        totals.dirs += 1;
        for s in 0..MULTI_SUBDIRS {
            let dir = root.join(format!("d{d:02}/s{s:02}"));
            totals.bytes += seed_multi_leaf_dir(&dir, next_size);
            totals.files += MULTI_FILES as u64;
            totals.dirs += 1;
            next_size += MULTI_FILES as u64;
        }
    }
    totals
}

fn run_multi_level_analyzer(root: &Path) -> (AnalyzerResult, Duration) {
    let start = Instant::now();
    let res = analyze_path(
        Some(&root.to_string_lossy()),
        Arc::new(AtomicBool::new(false)),
        None,
        AnalyzerOptions {
            worker_threads: 4,
            max_files_per_dir: CAP,
            max_children_per_dir: CAP,
            ..AnalyzerOptions::default()
        },
    )
    .expect("analyze multi-level sandbox");
    let elapsed = start.elapsed();
    eprintln!(
        "ISSUE97_ANALYZER_MULTI: files={} dirs={} elapsed_ms={} retained_children={}",
        res.progress.scanned_files,
        res.progress.scanned_dirs,
        elapsed.as_millis(),
        res.root.children.len()
    );
    (res, elapsed)
}

fn assert_multi_level_totals(res: &AnalyzerResult, expected: &MultiTreeTotals, elapsed: Duration) {
    assert!(
        elapsed < Duration::from_secs(30),
        "multi-level analyzer exceeded time budget: {elapsed:?}"
    );
    assert_eq!(
        res.progress.scanned_files, expected.files,
        "every file across every level must be scanned"
    );
    assert_eq!(
        res.progress.scanned_dirs, expected.dirs,
        "every directory in the tree must be scanned"
    );
    assert_eq!(
        res.progress.scanned_bytes, expected.bytes,
        "progress bytes must remain exact across levels"
    );
    assert_eq!(
        res.root.size_bytes, expected.bytes,
        "recursive size must remain exact across levels"
    );
    assert!(
        res.root.children.len() <= CAP,
        "root must retain at most {CAP} children, got {}",
        res.root.children.len()
    );
}

fn assert_multi_level_leaf_pruning(root: &AnalyzerNode) {
    // The largest leaf (last dir/subdir) is always retained; its files have
    // the last 16 unique sizes.
    let leaf = find_node_by_suffix(root, "d19\\s11")
        .or_else(|| find_node_by_suffix(root, "d19/s11"))
        .expect("largest leaf directory must be present");
    assert!(leaf.children.len() <= CAP, "leaf children must be bounded");
    let top_sizes: Vec<u64> = leaf.children.iter().map(|c| c.size_bytes).collect();
    let first_size =
        (1 + ((MULTI_DIRS - 1) * MULTI_SUBDIRS + (MULTI_SUBDIRS - 1)) * MULTI_FILES) as u64;
    let expected_top: Vec<u64> = (first_size + (MULTI_FILES - CAP) as u64
        ..first_size + MULTI_FILES as u64)
        .rev()
        .collect();
    assert_eq!(
        top_sizes, expected_top,
        "leaf must retain the largest files (leaf path={})",
        leaf.path
    );
}

#[test]
fn analyzer_prunes_large_multi_level_tree_within_budget_and_exact_totals() {
    let sb = Sandbox::new();
    let root = sb.create_dir("multi").unwrap();
    let expected = seed_multi_level_tree(&root);

    let (res, elapsed) = run_multi_level_analyzer(&root);
    assert_multi_level_totals(&res, &expected, elapsed);
    assert_multi_level_leaf_pruning(&res.root);
    assert_sentinel_intact(&sb);
}

fn make_item(id: &str, root: &Path, path: &Path, size: u64) -> remover::ValidatedItem {
    let root_canon = CanonicalPath::from_existing_path(&root.to_string_lossy()).unwrap();
    let path_canon = CanonicalPath::from_existing_path(&path.to_string_lossy())
        .map(|v| v.into_canonical())
        .unwrap_or_else(|_| CanonicalPath::parse_lexical(&path.to_string_lossy()).unwrap());
    remover::validated_item_for_tests(
        id,
        "issue97-target",
        root_canon.as_str().to_string(),
        path_canon.as_str().to_string(),
        size,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    )
}

fn clean_options(journal_dir: &Path, estimated: u64) -> CleanOptions {
    CleanOptions {
        preview_estimated_bytes: estimated,
        min_age: Duration::from_secs(3600),
        now: now(),
        journal_base_dir: Some(journal_dir.to_path_buf()),
    }
}

#[test]
fn clean_revalidates_many_files_with_cached_ancestor_chains() {
    let sb = Sandbox::new();
    // A deep path forces long ancestor chains to be revalidated for every item.
    let target = sb
        .create_dir("target/level0/level1/level2/level3/level4/level5")
        .unwrap();
    let journal = sb.create_dir("journal").unwrap();
    let canary = CanaryTree::seed_external("issue97_clean_many").unwrap();

    const FILES: usize = 1000;
    let mut items = Vec::with_capacity(FILES);
    let mut total = 0u64;
    for i in 0..FILES {
        let name = format!("f{i:05}.bin");
        let path = target.join(&name);
        fs::write(&path, b"payload").unwrap();
        set_filetime(&path, old_mtime_secs());
        let size = fs::metadata(&path).unwrap().len();
        total += size;
        items.push(make_item(&format!("i{i}"), &target, &path, size));
    }

    let start = Instant::now();
    let report = remover::execute_clean(items, clean_options(&journal, total)).unwrap();
    let elapsed = start.elapsed();
    eprintln!(
        "ISSUE97_CLEAN: files={} deleted={} elapsed_ms={}",
        FILES,
        report.bytes_freed,
        elapsed.as_millis()
    );

    assert!(
        elapsed < Duration::from_secs(90),
        "clean exceeded time budget: {elapsed:?}"
    );
    assert_eq!(report.items.len(), FILES, "every item must be reported");
    assert!(
        report.items.iter().all(|r| r.status == "deleted"),
        "every valid item must be revalidated and deleted"
    );
    assert_eq!(report.bytes_freed, total, "all bytes must be freed");
    assert!(
        canary.verify().is_ok(),
        "external canary must remain intact"
    );
    assert_sentinel_intact(&sb);
}

fn make_tamper_item(target: &Path, victim: &Path) -> remover::ValidatedItem {
    let victim_canon = CanonicalPath::from_existing_path(&victim.to_string_lossy()).unwrap();
    let target_canon = CanonicalPath::from_existing_path(&target.to_string_lossy()).unwrap();
    remover::validated_item_for_tests(
        "tampered-1",
        "issue97-target",
        target_canon.as_str().to_string(),
        victim_canon.as_str().to_string(),
        12,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    )
}

/// Prime the per-directory ancestor-chain cache with a safe clean in the
/// victim's directory level.
fn warm_tamper_cache(target: &Path, journal: &Path) {
    let warm_victim = target.join("warm.bin");
    fs::write(&warm_victim, b"warm").unwrap();
    set_filetime(&warm_victim, old_mtime_secs());
    let warm_canon = CanonicalPath::from_existing_path(&warm_victim.to_string_lossy()).unwrap();
    let target_canon = CanonicalPath::from_existing_path(&target.to_string_lossy()).unwrap();
    let warm_item = remover::validated_item_for_tests(
        "warm-1",
        "issue97-target",
        target_canon.as_str().to_string(),
        warm_canon.as_str().to_string(),
        4,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    );
    let report = remover::execute_clean(vec![warm_item], clean_options(journal, 4)).unwrap();
    assert_eq!(report.bytes_freed, 4, "warm-up clean must succeed");
}

/// Replace the Target root with a Link Point to a mirror that still holds a
/// file at the victim path, so the item cannot be rejected as merely missing.
fn tamper_target_with_mirror(sb: &Sandbox, target: &Path, victim: &Path) {
    let mirror = sb.create_dir("mirror").unwrap();
    let mirrored_victim = mirror.join("victim.bin");
    fs::write(&mirrored_victim, b"must survive").unwrap();
    set_filetime(&mirrored_victim, old_mtime_secs());
    fs::remove_dir_all(target).unwrap();
    sweeper_testkit::create_junction(target, &mirror).unwrap();
    assert!(victim.exists(), "victim path must survive the tamper");
}

fn assert_tamper_rejected(report: &remover::CleanReport, victim: &Path) {
    assert_eq!(
        report.bytes_freed, 0,
        "tampered target must cause zero deletions: {report:?}"
    );
    assert!(
        report.aborted || report.skipped_count > 0,
        "tampered target must be rejected"
    );
    let reasons = report
        .items
        .iter()
        .map(|r| r.reason.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        reasons.contains("link point"),
        "rejection must come from Link-Point detection, not a missing file: {reasons}"
    );
    assert_eq!(
        fs::read(victim).unwrap(),
        b"must survive",
        "the victim must remain byte-for-byte intact"
    );
}

#[test]
fn clean_revalidation_rejects_tampered_target_with_cached_ancestor_chains() {
    let sb = Sandbox::new();
    let target = sb.create_dir("target/deep/chain").unwrap();
    let journal = sb.create_dir("journal").unwrap();
    let canary = CanaryTree::seed_external("issue97_tampered").unwrap();

    let victim = target.join("victim.bin");
    fs::write(&victim, b"must survive").unwrap();
    set_filetime(&victim, old_mtime_secs());
    let item = make_tamper_item(&target, &victim);

    warm_tamper_cache(&target, &journal);
    tamper_target_with_mirror(&sb, &target, &victim);

    let report = remover::execute_clean(vec![item], clean_options(&journal, 12)).unwrap();
    assert_tamper_rejected(&report, &victim);
    assert!(
        canary.verify().is_ok(),
        "external canary must remain intact"
    );
    assert_sentinel_intact(&sb);
}
