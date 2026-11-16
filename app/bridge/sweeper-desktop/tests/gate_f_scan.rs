mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use sweeper_contract::{ContractError, ScanProgressEvent};
use sweeper_desktop::commands_impl;
use sweeper_testkit::Sandbox;

fn assert_session_invalid(err: ContractError) {
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn unknown_item_ids_rejected() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    common::seed_session(&state, &root, &f, 1_700_000_000);
    let err = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["missing".to_string()],
        None,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::UnknownItem { .. }), "{err:?}");
}

#[test]
fn expired_session_after_15_minutes_rejected() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    common::seed_session(&state, &root, &f, 1_700_000_000);
    state.clock.advance_secs(15 * 60 + 1);
    let err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    assert_session_invalid(err);
    let err = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap_err();
    assert_session_invalid(err);
}

#[test]
fn invalidated_session_on_settings_generation_change_rejected() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    common::seed_session(&state, &root, &f, 1_700_000_000);
    let mut settings = state.settings.lock().unwrap();
    let mut s = settings.get();
    s.min_age_hours = 10;
    let _ = settings.update(s);
    drop(settings);
    let err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    assert_session_invalid(err);
}

#[test]
fn start_scan_emits_started_progress_completed_with_expected_session_and_decimal_counters() {
    let sandbox_scan = Sandbox::new();
    let scan_dir = sandbox_scan.create_dir("ScanRoot").unwrap();
    let _ = sandbox_scan
        .write_file("ScanRoot\\progress.tmp", b"progress-data")
        .unwrap();
    let state = common::state_with_temp_settings(|s| {
        s.enable_tier1 = false;
        s.enable_tier2 = false;
        s.enable_tier3 = false;
        s.custom_paths = vec![scan_dir.to_string_lossy().to_string()];
    });
    let events = Arc::new(std::sync::Mutex::new(Vec::<ScanProgressEvent>::new()));
    let events_cb = events.clone();
    let resp = commands_impl::start_scan(
        &state,
        None,
        Some(move |evt: ScanProgressEvent| events_cb.lock().unwrap().push(evt)),
    )
    .expect("start_scan should succeed");
    let events = events.lock().unwrap().clone();
    common::assert_scan_progress_events(&events, &resp);
    let results = commands_impl::get_scan_results(&state, resp.session_id.clone()).unwrap();
    common::assert_scan_counters_decimal_strings(&results);
}

#[test]
fn session_store_lock_not_held_during_inflight_scan() {
    let sandbox_scan = Sandbox::new();
    let scan_dir = sandbox_scan.create_dir("ScanRoot").unwrap();
    let _ = sandbox_scan
        .write_file("ScanRoot\\locktest.tmp", b"lock-scan-data")
        .unwrap();
    let state = Arc::new(common::state_with_temp_settings(|s| {
        s.enable_tier1 = false;
        s.enable_tier2 = false;
        s.enable_tier3 = false;
        s.custom_paths = vec![scan_dir.to_string_lossy().to_string()];
    }));
    let progress_count = Arc::new(AtomicUsize::new(0));
    let lock_ok = Arc::new(AtomicBool::new(true));
    let resp = commands_impl::start_scan(
        &state,
        None,
        Some({
            let state = Arc::clone(&state);
            let progress_count = Arc::clone(&progress_count);
            let lock_ok = Arc::clone(&lock_ok);
            move |evt: ScanProgressEvent| {
                if matches!(evt, ScanProgressEvent::Progress { .. }) {
                    progress_count.fetch_add(1, Ordering::SeqCst);
                    if state.sessions.try_lock().is_err() {
                        lock_ok.store(false, Ordering::SeqCst);
                    }
                }
            }
        }),
    )
    .expect("start_scan should succeed");
    assert!(!resp.session_id.is_empty());
    assert!(progress_count.load(Ordering::SeqCst) >= 1);
    assert!(lock_ok.load(Ordering::SeqCst));
}
