//! Issue #135 — the log/summary path double-counts Tier 3 / WholeCache root bytes.
//!
//! A root removal writes one `deleted` outcome per child (child bytes) plus a
//! root outcome whose `bytes` repeats the sum of those children. Any consumer
//! that sums every `deleted` outcome therefore reports ~2x the reclaimed bytes.
//! The authoritative `CleanReport` is unaffected; this test guards the emitted
//! journal outcomes only.

#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup.

use std::fs;
use std::path::Path;
use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::Sandbox;

// This shared module exposes helpers that other test binaries use; this test
// only needs a subset, so suppress dead_code locally rather than weakening the
// workspace lint settings or changing tests/common/mod.rs.
#[allow(dead_code)]
mod common;
use common::{assert_sentinel_intact, canonical_str, now, write_file};

/// Bytes from one journal line when it is a `deleted` outcome.
fn deleted_bytes_in_line(line: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if value.get("record").and_then(|r| r.as_str()) != Some("outcome") {
        return None;
    }
    if value.get("status").and_then(|s| s.as_str()) != Some("deleted") {
        return None;
    }
    Some(value.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0))
}

/// Bytes recorded by every `deleted` outcome in the journal for `journal_dir`.
fn deleted_outcome_bytes(journal_dir: &Path) -> Vec<u64> {
    let log_dir = sweeper_core::journal::resolve_log_dir(journal_dir);
    let Ok(entries) = fs::read_dir(log_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .flat_map(|contents| {
            contents
                .lines()
                .filter_map(deleted_bytes_in_line)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn whole_cache_root_outcome_does_not_repeat_child_bytes() {
    let sb = Sandbox::new();
    let cache_root = sb.create_dir("uv-cache-135").unwrap();
    write_file(&cache_root.join("a.bin"), b"aaaa");
    write_file(&cache_root.join("b.bin"), b"bb");
    let child_bytes = 6u64; // 4 + 2

    let item = remover::ValidatedItem::from_whole_cache_root(
        "issue135-wc",
        "uv-cache-135",
        canonical_str(&cache_root),
        child_bytes,
    )
    .expect("whole-cache root must validate");

    let journal_dir = sb.create_dir("journal").unwrap();
    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(24 * 3600),
            now: now(),
            journal_base_dir: Some(journal_dir.clone()),
        },
    )
    .unwrap();

    assert!(!report.aborted, "aborted: {:?}", report.abort_reason);
    assert!(!cache_root.exists(), "whole-cache root must be removed");

    // CleanReport accounting must stay correct and unchanged by #135.
    assert_eq!(report.bytes_freed, child_bytes);

    let outcomes = deleted_outcome_bytes(&journal_dir);
    assert!(
        outcomes.len() >= 2,
        "expected per-child deleted outcomes, got {outcomes:?}"
    );
    let summed: u64 = outcomes.iter().fold(0u64, |a, b| a.saturating_add(*b));
    assert_eq!(
        summed, child_bytes,
        "deleted outcomes must not repeat the root's child-byte total: {outcomes:?} sum to {summed}"
    );
    assert_sentinel_intact(&sb);
}
