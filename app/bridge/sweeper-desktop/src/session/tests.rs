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
