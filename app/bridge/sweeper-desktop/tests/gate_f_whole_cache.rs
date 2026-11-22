//! Issue #41 — end-to-end bridge test: a whole-cache scan item must preview and
//! clean as a single all-or-nothing unit, deleting the entire cache root while
//! preserving everything outside the Target and the external canary.

mod common;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use sweeper_contract::{
    ExecuteCleanResponse, RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto,
    U64String,
};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::{StoredItem, TestScanSessionSeed};
use sweeper_desktop::AppState;
use sweeper_testkit::{CanaryTree, Sandbox};

const SESSION_ID: &str = "scan-test";
const ITEM_ID: &str = "t1::1";
const CACHE_TARGET_ID: &str = "uv-cache";
const WHOLE_CACHE_SIZE: u64 = 1_000_000;

fn canonical_str(path: &Path) -> String {
    sweeper_core::paths::CanonicalPath::from_existing_path(&path.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

/// The single item a whole-cache scan emits. Its size is a realistic recursive
/// estimate: a directory's own metadata length is 0 on Windows and would trip
/// the 120% circuit breaker.
fn whole_cache_scan_item(canonical: &str) -> ScanItemDto {
    ScanItemDto {
        id: ITEM_ID.to_string(),
        target_id: CACHE_TARGET_ID.to_string(),
        path: canonical.to_string(),
        size: U64String(WHOLE_CACHE_SIZE),
        mtime_unix_secs: U64String(1_700_000_000),
        age_eligible: true,
    }
}

fn whole_cache_target_result(canonical: &str, item: &ScanItemDto) -> TargetScanResultDto {
    TargetScanResultDto {
        target_id: CACHE_TARGET_ID.to_string(),
        target_name: "uv Cache".to_string(),
        tier: TierDto::Tier2,
        risk: RiskDto::Caution,
        resolved_paths: vec![canonical.to_string()],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(WHOLE_CACHE_SIZE),
        eligible_bytes: U64String(WHOLE_CACHE_SIZE),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    }
}

fn stored_whole_cache_item(item: ScanItemDto) -> StoredItem {
    StoredItem {
        dto: item,
        target_id: CACHE_TARGET_ID.to_string(),
        target_name: "uv Cache".to_string(),
        tier: sweeper_core::Tier::Tier2,
        risk: sweeper_core::Risk::Caution,
        age_eligible: true,
        requires_typed_confirmation: false,
    }
}

fn seed_whole_cache_session(state: &AppState<FixedClock>, canonical: &str) {
    let item = whole_cache_scan_item(canonical);
    let target = whole_cache_target_result(canonical, &item);
    let results = ScanResultsDto {
        session_id: SESSION_ID.to_string(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(ITEM_ID.to_string(), stored_whole_cache_item(item));
    let settings_gen = state.settings.lock().unwrap().generation();
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(TestScanSessionSeed {
            now: 1_800_000_000,
            settings_generation: settings_gen,
            session_id: SESSION_ID.to_string(),
            results,
            items,
        });
}

fn preview_and_clean(state: &AppState<FixedClock>) -> ExecuteCleanResponse {
    let preview = commands_impl::preview_clean(
        state,
        SESSION_ID.to_string(),
        vec![ITEM_ID.to_string()],
        None,
    )
    .expect("whole-cache item must preview");
    let logs = tempfile::tempdir().unwrap();
    commands_impl::execute_clean(
        state,
        preview.preview_token,
        Some(logs.path().to_path_buf()),
        |_| {},
    )
    .expect("whole-cache clean must execute")
}

fn assert_clean_effects(
    resp: &ExecuteCleanResponse,
    cache_root: &Path,
    bystander: &Path,
    canary: &CanaryTree,
    sandbox: &Sandbox,
) {
    assert!(!resp.aborted, "abort_reason={:?}", resp.abort_reason);
    assert!(!cache_root.exists(), "whole cache root must be removed");
    assert!(bystander.exists(), "non-targeted file must survive");
    canary.verify().expect("canary must remain intact");
    sandbox.assert_sentinel_present();
}

#[test]
fn whole_cache_scan_item_previews_and_cleans_entire_cache_root() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("gate_f_whole_cache").unwrap();
    let cache_root = sandbox.create_dir("uv-cache").unwrap();
    let nested = cache_root.join("v1");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("a.bin"), b"aaa").unwrap();
    let bystander = sandbox.write_file("bystander\\keep.txt", b"keep").unwrap();

    let canonical = canonical_str(&cache_root);
    seed_whole_cache_session(&state, &canonical);
    let resp = preview_and_clean(&state);
    assert_clean_effects(&resp, &cache_root, &bystander, &canary, &sandbox);
}
