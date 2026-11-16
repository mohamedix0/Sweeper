use std::sync::atomic::Ordering;
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

struct CustomSessionSeed {
    session_id: String,
    item_id: String,
    target_id: String,
    target_name: String,
    path: String,
    resolved_paths: Vec<String>,
    tier: sweeper_contract::TierDto,
    risk: sweeper_contract::RiskDto,
    core_tier: sweeper_core::Tier,
    core_risk: sweeper_core::Risk,
}

fn seed_custom_path_session(store: &mut SessionStore, seed: CustomSessionSeed) {
    use std::collections::HashMap;
    use sweeper_contract::{ScanItemDto, ScanResultsDto, TargetScanResultDto, U64String};
    let item = ScanItemDto {
        id: seed.item_id.clone(),
        target_id: seed.target_id.clone(),
        path: seed.path.clone(),
        size: U64String(3),
        mtime_unix_secs: U64String(1_700_000_000),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: seed.target_id.clone(),
        target_name: seed.target_name.clone(),
        tier: seed.tier,
        risk: seed.risk,
        resolved_paths: seed.resolved_paths,
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
            dto: item,
            target_id: seed.target_id,
            target_name: seed.target_name,
            tier: seed.core_tier,
            risk: seed.core_risk,
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

fn stale_seed(session_id: &str, item_id: &str, root: &str) -> CustomSessionSeed {
    use sweeper_contract::{RiskDto, TierDto};
    CustomSessionSeed {
        session_id: session_id.to_string(),
        item_id: item_id.to_string(),
        target_id: "stale-rust-target".to_string(),
        target_name: "Stale Rust target/".to_string(),
        path: root.to_string(),
        resolved_paths: vec![root.to_string()],
        tier: TierDto::Tier3,
        risk: RiskDto::Caution,
        core_tier: sweeper_core::Tier::Tier3,
        core_risk: sweeper_core::Risk::Caution,
    }
}

fn bad_contents_seed() -> CustomSessionSeed {
    use sweeper_contract::{RiskDto, TierDto};
    CustomSessionSeed {
        session_id: "scan-bad".to_string(),
        item_id: "bad::1".to_string(),
        target_id: "user-temp".to_string(),
        target_name: "User Temp".to_string(),
        path: r"C:\Other\file.tmp".to_string(),
        resolved_paths: vec![r"C:\Users\X\AppData\Local\Temp".to_string()],
        tier: TierDto::Tier1,
        risk: RiskDto::Safe,
        core_tier: sweeper_core::Tier::Tier1,
        core_risk: sweeper_core::Risk::Safe,
    }
}

#[test]
fn preview_and_clean_stale_directory_item_equal_root() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("target");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("junk.bin"), b"abc").unwrap();
    let root_canon = std::fs::canonicalize(&root).unwrap();
    let mut root_str = root_canon.to_string_lossy().to_string();
    if let Some(stripped) = root_str.strip_prefix(r"\\?\UNC\") {
        root_str = format!(r"\\{stripped}");
    } else if let Some(stripped) = root_str.strip_prefix(r"\\?\") {
        root_str = stripped.to_string();
    }
    seed_custom_path_session(&mut store, stale_seed("scan-stale", "stale::1", &root_str));
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
    let journal_dir = tempfile::tempdir().unwrap();
    let exec = store
        .execute_clean(
            &clock,
            super::clean::ExecuteCleanRequest {
                preview_token: preview.preview_token,
                current_settings_generation: 1,
                journal_base_dir: Some(journal_dir.path().to_path_buf()),
            },
            |_| {},
        )
        .unwrap();
    assert!(!exec.aborted, "{exec:?}");
}

#[test]
fn preview_rejects_non_descendant_contents_only_target() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    seed_custom_path_session(&mut store, bad_contents_seed());
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

#[test]
fn cancel_analyzer_sets_cancellation_flag_for_active_session() {
    let clock = FixedClock::new(1_800_000_000);
    let mut store = SessionStore::new();
    let prepared = store.begin_analyzer(&clock, 1).unwrap();
    assert!(!prepared.cancel.load(Ordering::Relaxed));

    let cancelled = store.cancel_analyzer(&prepared.session_id);
    assert!(cancelled, "active analyzer session should be cancellable");
    assert!(
        prepared.cancel.load(Ordering::Relaxed),
        "cancel_analyzer must set the active session cancellation flag"
    );
}
