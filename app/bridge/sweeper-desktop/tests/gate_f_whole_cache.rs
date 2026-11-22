//! Issue #41 — end-to-end bridge test: a whole-cache scan item must preview and
//! clean as a single all-or-nothing unit, deleting the entire cache root while
//! preserving everything outside the Target and the external canary.

mod common;

use std::collections::HashMap;
use std::fs;

use sweeper_contract::{
    RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto, U64String,
};
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::{StoredItem, TestScanSessionSeed};
use sweeper_testkit::{CanaryTree, Sandbox};

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

    let canonical =
        sweeper_core::paths::CanonicalPath::from_existing_path(&cache_root.to_string_lossy())
            .unwrap()
            .as_str()
            .to_string();
    // Seed a whole-cache item with a realistic recursive size estimate (a
    // directory's own metadata length is 0 on Windows and would trip the 120%
    // circuit breaker). This mirrors what the whole-cache Scanner emits.
    let item = ScanItemDto {
        id: "t1::1".to_string(),
        target_id: "uv-cache".to_string(),
        path: canonical.clone(),
        size: U64String(1_000_000),
        mtime_unix_secs: U64String(1_700_000_000),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: "uv-cache".to_string(),
        target_name: "uv Cache".to_string(),
        tier: TierDto::Tier2,
        risk: RiskDto::Caution,
        resolved_paths: vec![canonical.clone()],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(1_000_000),
        eligible_bytes: U64String(1_000_000),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    };
    let results = ScanResultsDto {
        session_id: "scan-test".to_string(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(
        item.id.clone(),
        StoredItem {
            dto: item,
            target_id: "uv-cache".to_string(),
            target_name: "uv Cache".to_string(),
            tier: sweeper_core::Tier::Tier2,
            risk: sweeper_core::Risk::Caution,
            age_eligible: true,
            requires_typed_confirmation: false,
        },
    );
    let settings_gen = state.settings.lock().unwrap().generation();
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(TestScanSessionSeed {
            now: 1_800_000_000,
            settings_generation: settings_gen,
            session_id: "scan-test".to_string(),
            results,
            items,
        });

    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .expect("whole-cache item must preview");
    let logs = tempfile::tempdir().unwrap();
    let resp = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs.path().to_path_buf()),
        |_| {},
    )
    .expect("whole-cache clean must execute");

    assert!(!resp.aborted, "abort_reason={:?}", resp.abort_reason);
    assert!(!cache_root.exists(), "whole cache root must be removed");
    assert!(bystander.exists(), "non-targeted file must survive");
    canary.verify().expect("canary must remain intact");
    sandbox.assert_sentinel_present();
}
