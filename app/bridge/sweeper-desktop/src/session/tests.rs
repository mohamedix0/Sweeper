use std::sync::{Arc, Mutex};

use sweeper_contract::{ContractError, ScanProgressEvent};
use sweeper_core::scanner::ScanReport;
use sweeper_core::settings::SweeperSettings;

use crate::clock::FixedClock;

use super::scan::ScanRunRequest;
use super::support::find_containing_root;
use super::{ScanFinishInput, SessionStore};

impl SessionStore {
    fn is_scan_active(&self, session_id: &str) -> bool {
        self.active_scans.contains_key(session_id)
    }
}

fn empty_scan_report() -> ScanReport {
    ScanReport {
        started_unix_secs: 0,
        min_age_hours: 24,
        targets: Vec::new(),
    }
}

#[test]
fn find_root_prefers_longest_match() {
    let roots = vec![
        r"C:\Users\A\AppData\Local".to_string(),
        r"C:\Users\A\AppData\Local\Temp".to_string(),
    ];
    let found = find_containing_root(&roots, r"C:\Users\A\AppData\Local\Temp\f.txt");
    assert_eq!(found.as_deref(), Some(r"C:\Users\A\AppData\Local\Temp"));
}

#[test]
fn finish_scan_settings_mismatch_clears_active_tracking() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    let settings = SweeperSettings::default();
    let prepared = store
        .begin_scan(
            &clock,
            super::scan::ScanBeginRequest {
                tiers: None,
                settings: settings.clone(),
                settings_generation: 1,
            },
        )
        .unwrap();
    assert!(store.is_scan_active(&prepared.session_id));

    let err = store
        .finish_scan(
            &clock,
            ScanFinishInput {
                session_id: prepared.session_id.clone(),
                settings_generation_at_start: 1,
                current_settings_generation: 2,
                effective_min_age_hours: settings.min_age_hours,
                report: empty_scan_report(),
            },
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ContractError::SessionInvalid { ref reason } if reason == "session invalidated by settings change"
    ));
    assert!(
        !store.is_scan_active(&prepared.session_id),
        "active scan tracking must be cleared when finish_scan is rejected"
    );
}

#[test]
fn start_scan_with_progress_emits_failed_terminal_event_on_finish_settings_mismatch() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: false,
        ..SweeperSettings::default()
    };

    let events: Arc<Mutex<Vec<ScanProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_cb = events.clone();
    let err = store
        .start_scan_with_progress(
            &clock,
            ScanRunRequest {
                tiers: None,
                settings,
                settings_generation_at_start: 1,
                current_settings_generation: 2,
            },
            Some(move |evt: ScanProgressEvent| {
                events_cb.lock().unwrap().push(evt);
            }),
        )
        .unwrap_err();

    assert!(matches!(
        err,
        ContractError::SessionInvalid { ref reason } if reason == "session invalidated by settings change"
    ));

    let events = events.lock().unwrap();
    assert!(
        matches!(events.first(), Some(ScanProgressEvent::Started { .. })),
        "expected Started event, got: {events:?}"
    );
    match events.last() {
        Some(ScanProgressEvent::Failed { session_id, error }) => {
            assert!(!session_id.is_empty());
            assert_eq!(error, &err);
        }
        other => panic!("expected terminal Failed event, got: {other:?} (all: {events:?})"),
    }
    assert!(
        events
            .iter()
            .all(|e| !matches!(e, ScanProgressEvent::Completed { .. })),
        "must not emit Completed when finish_scan fails: {events:?}"
    );
}

struct StaleSessionSeed {
    session_id: String,
    item_id: String,
    root_str: String,
}

fn seed_stale_dir_session(store: &mut SessionStore, seed: StaleSessionSeed) {
    use std::collections::HashMap;
    use sweeper_contract::{
        RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto, U64String,
    };
    let item = ScanItemDto {
        id: seed.item_id.clone(),
        target_id: "stale-rust-target".to_string(),
        path: seed.root_str.clone(),
        size: U64String(3),
        mtime_unix_secs: U64String(1_700_000_000),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: "stale-rust-target".to_string(),
        target_name: "Stale Rust target/".to_string(),
        tier: TierDto::Tier3,
        risk: RiskDto::Caution,
        resolved_paths: vec![seed.root_str.clone()],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(3),
        eligible_bytes: U64String(3),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    };
    let results = ScanResultsDto {
        session_id: seed.session_id.clone(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(
        item.id.clone(),
        super::StoredItem {
            dto: item.clone(),
            target_id: "stale-rust-target".to_string(),
            target_name: "Stale Rust target/".to_string(),
            tier: sweeper_core::Tier::Tier3,
            risk: sweeper_core::Risk::Caution,
            age_eligible: true,
            requires_typed_confirmation: false,
        },
    );
    store.create_test_session_with_items(super::TestScanSessionSeed {
        now: 1_800_000_000,
        settings_generation: 1,
        session_id: seed.session_id,
        results,
        items,
    });
}

#[test]
fn preview_and_clean_stale_directory_item_equal_root() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("target");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("junk.bin"), b"abc").unwrap();
    seed_stale_dir_session(
        &mut store,
        StaleSessionSeed {
            session_id: "scan-stale".to_string(),
            item_id: "stale::1".to_string(),
            root_str: root.to_string_lossy().to_string(),
        },
    );
    let preview = store
        .preview_clean(
            &clock,
            super::PreviewCleanRequest {
                session_id: "scan-stale".to_string(),
                item_ids: vec!["stale::1".to_string()],
                confirmations: None,
                current_settings_generation: 1,
            },
        )
        .unwrap();
    let exec = store
        .execute_clean(
            &clock,
            super::clean::ExecuteCleanRequest {
                preview_token: preview.preview_token,
                current_settings_generation: 1,
                journal_base_dir: None,
            },
            |_| {},
        )
        .unwrap();
    assert!(!exec.aborted, "{exec:?}");
}

fn seed_bad_contents_session(store: &mut SessionStore) {
    use std::collections::HashMap;
    use sweeper_contract::{
        RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto, U64String,
    };
    let item = ScanItemDto {
        id: "bad::1".to_string(),
        target_id: "user-temp".to_string(),
        path: r"C:\Other\file.tmp".to_string(),
        size: U64String(1),
        mtime_unix_secs: U64String(1_700_000_000),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: "user-temp".to_string(),
        target_name: "User Temp".to_string(),
        tier: TierDto::Tier1,
        risk: RiskDto::Safe,
        resolved_paths: vec![r"C:\Users\X\AppData\Local\Temp".to_string()],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(1),
        eligible_bytes: U64String(1),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    };
    let results = ScanResultsDto {
        session_id: "scan-bad".to_string(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(
        item.id.clone(),
        super::StoredItem {
            dto: item.clone(),
            target_id: "user-temp".to_string(),
            target_name: "User Temp".to_string(),
            tier: sweeper_core::Tier::Tier1,
            risk: sweeper_core::Risk::Safe,
            age_eligible: true,
            requires_typed_confirmation: false,
        },
    );
    store.create_test_session_with_items(super::TestScanSessionSeed {
        now: 1_800_000_000,
        settings_generation: 1,
        session_id: "scan-bad".to_string(),
        results,
        items,
    });
}

#[test]
fn preview_rejects_non_descendant_contents_only_target() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    seed_bad_contents_session(&mut store);
    let err = store
        .preview_clean(
            &clock,
            super::PreviewCleanRequest {
                session_id: "scan-bad".to_string(),
                item_ids: vec!["bad::1".to_string()],
                confirmations: None,
                current_settings_generation: 1,
            },
        )
        .unwrap_err();
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}
