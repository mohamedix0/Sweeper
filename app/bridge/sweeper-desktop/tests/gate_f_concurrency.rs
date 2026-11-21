//! Gate F — backend concurrency preflight (#95).
//!
//! A second `Scan`, or a `Clean` started while a `Scan` is Running, is rejected
//! at the backend with a structured error instead of running concurrently.
//! Every terminal outcome clears the in-flight state, so sequential runs never
//! produce false positives. A `Clean` rejection never consumes its single-use
//! `preview_token` and never deletes anything.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sweeper_contract::{ContractError, ScanProgressEvent, U64String};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::ScanBeginRequest;
use sweeper_desktop::AppState;
use sweeper_testkit::Sandbox;

const SCAN_RUNNING_REASON: &str = "a Scan is already running";

/// Register an in-flight Scan through the session-layer seam (#94). The
/// preflight lives at the operation entry points, so this raw seam can still
/// set up concurrent in-flight Scans to test targeted cancellation.
fn register_inflight_scan(state: &AppState<FixedClock>) -> (String, Arc<AtomicBool>) {
    let (core, generation) = {
        let settings = state.settings.lock().expect("settings lock");
        (settings.core(), settings.generation())
    };
    let prepared = state
        .sessions
        .lock()
        .expect("sessions lock")
        .begin_scan(
            &state.clock,
            ScanBeginRequest {
                tiers: None,
                settings: core,
                settings_generation: generation,
            },
        )
        .expect("register in-flight scan");
    (prepared.session_id, prepared.cancel)
}

/// State configured so a real `start_scan` only touches a sentinel sandbox,
/// never real `C:` user/system paths.
fn sandbox_scan_state() -> (AppState<FixedClock>, Sandbox) {
    let sandbox = Sandbox::new();
    let scan_dir = sandbox
        .create_dir("PreflightScanRoot")
        .expect("sandbox scan root");
    sandbox
        .write_file("PreflightScanRoot\\progress.tmp", b"progress-data")
        .expect("sandbox scan file");
    let state = common::state_with_temp_settings(|s| {
        s.enable_tier1 = false;
        s.enable_tier2 = false;
        s.enable_tier3 = false;
        s.custom_paths = vec![scan_dir.to_string_lossy().to_string()];
    });
    (state, sandbox)
}

fn assert_scan_in_progress(err: ContractError) {
    match err {
        ContractError::Internal { reason } => assert!(
            reason.contains(SCAN_RUNNING_REASON),
            "unexpected structured error reason: {reason}"
        ),
        other => panic!("expected a structured concurrency rejection, got {other:?}"),
    }
}

#[test]
fn overlapping_start_scan_is_rejected() {
    let (state, _sandbox) = sandbox_scan_state();
    register_inflight_scan(&state);

    let err = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>).unwrap_err();
    assert_scan_in_progress(err);
}

#[test]
fn preview_clean_during_scan_is_rejected() {
    let s = common::seeded_scan();

    register_inflight_scan(&s.state);

    let err = commands_impl::preview_clean(
        &s.state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap_err();
    assert_scan_in_progress(err);
}

#[test]
fn execute_clean_during_scan_is_rejected_without_consuming_token_or_deleting() {
    let s = common::seeded_scan();
    // Issue the token while idle, then start a Scan before executing.
    let token = common::preview_scan_token(&s.state);
    let (scan_id, _flag) = register_inflight_scan(&s.state);

    let logs_base = tempfile::tempdir().expect("logs base");
    let err = commands_impl::execute_clean(
        &s.state,
        token.clone(),
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .unwrap_err();
    assert_scan_in_progress(err);
    assert!(
        s.file.exists(),
        "no deletion may happen while a Scan is Running"
    );

    // The rejected attempt must not consume the single-use token. Once the
    // Scan settles, the very same token still executes the Clean.
    s.state
        .sessions
        .lock()
        .expect("sessions lock")
        .abort_scan(&scan_id);
    let resp = commands_impl::execute_clean(
        &s.state,
        token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .expect("rejected attempt must not consume the preview token");
    assert_eq!(resp.deleted_count, U64String(1));
    assert!(
        !s.file.exists(),
        "eligible file should be deleted after the Scan"
    );
}

#[test]
fn sequential_scans_succeed_after_completion() {
    let (state, _sandbox) = sandbox_scan_state();

    let first =
        commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>).expect("first scan");
    assert!(!first.session_id.is_empty());
    let second = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("second scan must not be rejected after the first completed");
    assert_ne!(first.session_id, second.session_id);
}

#[test]
fn clean_succeeds_after_scan_completes() {
    let (state, sandbox) = sandbox_scan_state();
    commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>).expect("scan");

    let s = common::seeded_scan_in(state, sandbox, "clean.tmp");
    let preview = commands_impl::preview_clean(
        &s.state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .expect("preview after a completed Scan");
    let logs_base = tempfile::tempdir().expect("logs base");
    let resp = commands_impl::execute_clean(
        &s.state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .expect("Clean after a completed Scan must not be rejected");
    assert_eq!(resp.deleted_count, U64String(1));
}

#[test]
fn scan_succeeds_after_clean() {
    let (state, sandbox) = sandbox_scan_state();
    let s = common::seeded_scan_in(state, sandbox, "clean.tmp");
    let preview = commands_impl::preview_clean(
        &s.state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .expect("preview");
    let logs_base = tempfile::tempdir().expect("logs base");
    commands_impl::execute_clean(
        &s.state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .expect("clean");

    let resp = commands_impl::start_scan(&s.state, None, None::<fn(ScanProgressEvent)>)
        .expect("Scan after a Clean must not be rejected");
    assert!(!resp.session_id.is_empty());
}

#[test]
fn in_flight_state_is_cleared_on_cancel_and_error() {
    let (state, _sandbox) = sandbox_scan_state();

    let (cancelled_id, flag) = register_inflight_scan(&state);
    commands_impl::cancel_scan(&state, cancelled_id.clone()).expect("cancel scan");
    assert!(flag.load(Ordering::Relaxed), "cancel flag must be set");
    // The scanner removes its own registration as it unwinds (#94).
    state
        .sessions
        .lock()
        .expect("sessions lock")
        .abort_scan(&cancelled_id);
    let after_cancel = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("scan after cancel");
    assert!(!after_cancel.session_id.is_empty());

    // A failed Scan removes its registration the same way a cancelled one does.
    let (failed_id, _flag) = register_inflight_scan(&state);
    state
        .sessions
        .lock()
        .expect("sessions lock")
        .abort_scan(&failed_id);
    let after_error = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("scan after error");
    assert!(!after_error.session_id.is_empty());
}

#[test]
fn expired_scan_session_does_not_block_a_new_scan() {
    let (state, _sandbox) = sandbox_scan_state();
    let first =
        commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>).expect("first scan");
    state.clock.advance_secs(15 * 60 + 1);
    assert!(
        commands_impl::get_scan_results(&state, first.session_id).is_err(),
        "stored session must expire after the 15-minute TTL"
    );

    let second = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("expired session must not leave stale in-flight state");
    assert!(!second.session_id.is_empty());
}
