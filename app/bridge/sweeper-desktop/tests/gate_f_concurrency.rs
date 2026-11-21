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

    // Both terminal outcomes are driven through the real `start_scan` entry
    // point so the production terminal handler (`handle_scan_terminal`) clears
    // the in-flight registration ΓÇö the test must not clean up after itself.

    // Cancel: flag the freshly started Scan from its own progress callback. The
    // callback runs after `begin_scan_checked` released the session lock, so
    // `cancel_scan` can acquire it without deadlocking.
    let cancel_state = state.clone();
    let cancelled_once = Arc::new(AtomicBool::new(false));
    let cancel_seen = cancelled_once.clone();
    let cancel_err = commands_impl::start_scan(
        &state,
        None,
        Some(move |event| {
            if let ScanProgressEvent::Started { session_id } = event {
                if !cancel_seen.swap(true, Ordering::Relaxed) {
                    commands_impl::cancel_scan(&cancel_state, session_id)
                        .expect("cancel from progress callback");
                }
            }
        }),
    )
    .unwrap_err();
    assert!(
        matches!(cancel_err, ContractError::ScanCancelled { .. }),
        "cancelled Scan must surface the typed cancellation error, got {cancel_err:?}"
    );
    assert!(
        cancelled_once.load(Ordering::Relaxed),
        "progress callback must have cancelled the Scan"
    );
    let after_cancel = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("scan after cancel must be admitted without manual cleanup");
    assert!(!after_cancel.session_id.is_empty());

    // Error: bump the scan-affecting generation mid-run so the commit fails and
    // takes the `Failed` terminal path instead of the cancellation path.
    let error_state = state.clone();
    let failed_once = Arc::new(AtomicBool::new(false));
    let failure_seen = failed_once.clone();
    let error_err = commands_impl::start_scan(
        &state,
        None,
        Some(move |event| {
            if let ScanProgressEvent::Started { .. } = event {
                if !failure_seen.swap(true, Ordering::Relaxed) {
                    let mut settings = error_state.settings.lock().expect("settings lock");
                    let mut dto = settings.get();
                    dto.min_age_hours = dto.min_age_hours.saturating_add(1);
                    settings.update(dto).expect("settings update in callback");
                }
            }
        }),
    )
    .unwrap_err();
    assert!(
        matches!(error_err, ContractError::SessionInvalid { .. }),
        "failed Scan commit must surface a session-invalid error, got {error_err:?}"
    );
    assert!(
        failed_once.load(Ordering::Relaxed),
        "progress callback must have changed settings"
    );
    let after_error = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("scan after error must be admitted without manual cleanup");
    assert!(!after_error.session_id.is_empty());
}

#[test]
fn invalidation_keeps_scan_registration_until_worker_terminates() {
    let (state, _sandbox) = sandbox_scan_state();
    let (scan_id, flag) = register_inflight_scan(&state);

    // A scan-affecting settings change flags the in-flight Scan for cancellation
    // but must not free the preflight slot while the worker may still be
    // touching the filesystem (#95).
    state
        .sessions
        .lock()
        .expect("sessions lock")
        .invalidate_scan_sessions();
    assert!(
        flag.load(Ordering::Relaxed),
        "invalidation must flag the Scan"
    );
    assert_scan_in_progress(
        commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>).unwrap_err(),
    );

    // The worker's terminal path clears the registration; only then is a new
    // Scan admitted.
    state
        .sessions
        .lock()
        .expect("sessions lock")
        .abort_scan(&scan_id);
    let resp = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("Scan admitted after the worker terminated");
    assert!(!resp.session_id.is_empty());
}

#[test]
fn invalidated_registration_expires_after_scan_ttl() {
    let (state, _sandbox) = sandbox_scan_state();
    register_inflight_scan(&state);
    state
        .sessions
        .lock()
        .expect("sessions lock")
        .invalidate_scan_sessions();

    // A worker that never reaches a terminal path must not block new runs
    // forever: cleanup reclaims the registration at the 15-minute session TTL.
    state.clock.advance_secs(15 * 60 + 1);
    let resp = commands_impl::start_scan(&state, None, None::<fn(ScanProgressEvent)>)
        .expect("stale registration must be reclaimed by the 15-minute TTL");
    assert!(!resp.session_id.is_empty());
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
