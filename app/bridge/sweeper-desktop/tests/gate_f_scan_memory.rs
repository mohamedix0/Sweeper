//! Gate F — bounded memory in Scan reporting and DTO construction (#96).
//!
//! Every path below is a synthetic fixture (`C:\Users\Alice\...`). No test in
//! this file touches a real user or system path, and no test performs a delete.

use std::time::{Duration, Instant};

use sweeper_contract::{RiskDto, TierDto, U64String};
use sweeper_core::scanner::{ScanItem, ScanReport, TargetScanResult};
use sweeper_core::{Risk, Tier};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::session::{build_prepared_scan_session, ScanFinishInput, SessionStore};

const ITEM_COUNT: usize = 20_000;
const BUDGET: Duration = Duration::from_secs(15);

fn synthetic_scan_report(item_count: usize) -> ScanReport {
    let items = (0..item_count)
        .map(|i| ScanItem {
            id: format!("ui-cache::item-{i}"),
            target_id: "ui-cache".to_string(),
            path: format!(r"C:\Users\Alice\AppData\Local\UI Cache\file-{i}.tmp"),
            file_identity: None,
            size: i as u64 + 1,
            mtime_unix_secs: 1_600_000_000 + i as u64,
            age_eligible: i % 2 == 0,
        })
        .collect::<Vec<_>>();
    let eligible = items
        .iter()
        .filter(|i| i.age_eligible)
        .fold((0u64, 0u64), |(bytes, count), i| {
            (bytes.saturating_add(i.size), count + 1)
        });
    ScanReport {
        started_unix_secs: 1_800_000_000,
        min_age_hours: 24,
        targets: vec![TargetScanResult {
            target_id: "ui-cache".to_string(),
            target_name: "UI Cache".to_string(),
            tier: Tier::Tier1,
            risk: Risk::Safe,
            resolved_paths: vec![r"C:\Users\Alice\AppData\Local\UI Cache".to_string()],
            scanned: true,
            status_reason: None,
            total_bytes: items.iter().map(|i| i.size).sum(),
            eligible_bytes: eligible.0,
            file_count: item_count as u64,
            eligible_count: eligible.1,
            skipped_count: 0,
            skipped_reasons: Default::default(),
            skipped_diagnostics: Vec::new(),
            items,
        }],
    }
}

#[test]
fn large_scan_builds_full_dto_tree_within_budget_and_preserves_results() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    let report = synthetic_scan_report(ITEM_COUNT);

    let start = Instant::now();

    // The large DTO tree is built from the report before the session store is
    // touched; only the finished tree is handed to `finish_scan` for insertion.
    let prepared = build_prepared_scan_session("scan-large", report);
    let expected = prepared.results.clone();
    assert_eq!(expected.targets.len(), 1);
    assert_eq!(expected.targets[0].items.len(), ITEM_COUNT);
    assert_eq!(
        expected.targets[0].items[0].id,
        "ui-cache::item-0".to_string()
    );
    assert_eq!(
        expected.targets[0].items[ITEM_COUNT - 1].id,
        format!("ui-cache::item-{}", ITEM_COUNT - 1)
    );

    store
        .finish_scan(
            &clock,
            ScanFinishInput {
                session_id: "scan-large".to_string(),
                settings_generation_at_start: 1,
                current_settings_generation: 1,
                effective_min_age_hours: 24,
                prepared,
            },
        )
        .expect("finish_scan");

    let results = store
        .get_scan_results(&clock, "scan-large", 1)
        .expect("stored session");

    let elapsed = start.elapsed();
    assert!(
        elapsed < BUDGET,
        "large Scan DTO build + commit took {elapsed:?} (budget {BUDGET:?})"
    );

    // Round-tripping through the store does not change a single DTO field.
    assert_eq!(results, expected);
    assert_eq!(
        results.targets[0].eligible_bytes,
        U64String(expected_eligible_bytes())
    );
    assert_eq!(results.targets[0].tier, TierDto::Tier1);
    assert_eq!(results.targets[0].risk, RiskDto::Safe);
}

#[test]
fn large_scan_results_are_independent_of_dto_build_call_order() {
    // Building the DTO tree twice from two identical reports must yield exactly
    // the same results: the mapping is deterministic and has no hidden state.
    let a = build_prepared_scan_session("scan-a", synthetic_scan_report(512));
    let b = build_prepared_scan_session("scan-b", synthetic_scan_report(512));
    // Session ids differ by design; compare the target trees only.
    assert_eq!(a.results.targets, b.results.targets);
    assert_eq!(a.items.len(), b.items.len());
    assert_eq!(a.target_roots, b.target_roots);
}

fn expected_eligible_bytes() -> u64 {
    // Even indices 0..ITEM_COUNT are eligible; size = index + 1.
    (0..ITEM_COUNT)
        .filter(|i| i % 2 == 0)
        .map(|i| i as u64 + 1)
        .sum()
}
