//! Gate F — session-scoped Scan cancellation and the `CloseRequested` hook.
//!
//! A cancel request sets a flag on exactly one in-flight `ScanSession`. A
//! window close reuses the #93 handshake and cancels in-flight Scans without
//! dropping finished sessions, Analyzer sessions, or preview tokens, and it
//! still waits for in-flight Cleans before exiting.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::close_gate::{CloseApproval, CloseDecision, CloseRequest};
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::ScanBeginRequest;
use sweeper_desktop::AppState;
use sweeper_testkit::Sandbox;

const SEAL_TIMEOUT: Duration = Duration::from_secs(2);

fn begin_scan(state: &AppState<FixedClock>) -> (String, Arc<AtomicBool>) {
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
        .expect("begin scan");
    (prepared.session_id, prepared.cancel)
}

fn begin_analyzer(state: &AppState<FixedClock>) -> Arc<AtomicBool> {
    let (scan_generation, analyzer_generation) = {
        let settings = state.settings.lock().expect("settings lock");
        (settings.generation(), settings.analyzer_generation())
    };
    state
        .sessions
        .lock()
        .expect("sessions lock")
        .begin_analyzer(&state.clock, scan_generation, analyzer_generation)
        .expect("begin analyzer")
        .cancel
}

fn request_close(state: &AppState<FixedClock>) -> String {
    match state.begin_close_request() {
        CloseRequest::DeferAndAsk { request_id } => request_id,
        other => panic!("expected DeferAndAsk, got {other:?}"),
    }
}

fn wait_for_clean_gate_sealed(state: &AppState<FixedClock>) {
    let deadline = Instant::now() + SEAL_TIMEOUT;
    while Instant::now() < deadline {
        if state.clean_gate.enter().is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("clean gate was not sealed within {SEAL_TIMEOUT:?}");
}

#[test]
fn cancel_scan_flags_only_the_targeted_session_and_is_idempotent() {
    let state = common::make_state();
    let (target_id, target_flag) = begin_scan(&state);
    let (other_id, other_flag) = begin_scan(&state);
    assert_ne!(target_id, other_id);

    commands_impl::cancel_scan(&state, target_id.clone()).expect("cancel");
    // Replaying the same request and cancelling an unknown id are both safe no-ops.
    commands_impl::cancel_scan(&state, target_id.clone()).expect("double cancel");
    commands_impl::cancel_scan(&state, "scan-unknown".to_string()).expect("unknown id");

    assert!(
        target_flag.load(Ordering::Relaxed),
        "targeted in-flight scan must be flagged"
    );
    assert!(
        !other_flag.load(Ordering::Relaxed),
        "unrelated in-flight scan must stay untouched"
    );
}

#[test]
fn cancelling_a_finished_or_expired_session_never_panics_or_disturbs_results() {
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").expect("sandbox root");
    let file = sandbox
        .write_file("Temp\\a.tmp", b"abc")
        .expect("sandbox file");
    let state = common::make_state();
    common::seed_session(&state, &root, &file, 1_700_000_000);

    commands_impl::cancel_scan(&state, "scan-test".to_string()).expect("finished session cancel");
    let results =
        commands_impl::get_scan_results(&state, "scan-test".to_string()).expect("session intact");
    assert_eq!(results.session_id, "scan-test");

    state.clock.advance_secs(15 * 60 + 1);
    commands_impl::cancel_scan(&state, "scan-test".to_string()).expect("expired session cancel");
    assert!(commands_impl::get_scan_results(&state, "scan-test".to_string()).is_err());
}

#[test]
fn close_request_cancels_in_flight_scans_without_nuking_unrelated_sessions() {
    let state = common::make_state();
    let (_first_id, first_flag) = begin_scan(&state);
    let (_second_id, second_flag) = begin_scan(&state);
    let analyzer_flag = begin_analyzer(&state);

    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").expect("sandbox root");
    let file = sandbox
        .write_file("Temp\\a.tmp", b"abc")
        .expect("sandbox file");
    common::seed_session(&state, &root, &file, 1_700_000_000);

    let request_id = request_close(&state);

    assert!(
        first_flag.load(Ordering::Relaxed) && second_flag.load(Ordering::Relaxed),
        "close must flag every in-flight Scan"
    );
    assert!(
        !analyzer_flag.load(Ordering::Relaxed),
        "close must not cancel Analyzer sessions"
    );
    assert_eq!(
        state.close_gate.pending_request().as_deref(),
        Some(request_id.as_str())
    );

    // A finished Scan session and its results survive the close request.
    let results =
        commands_impl::get_scan_results(&state, "scan-test".to_string()).expect("session intact");
    assert_eq!(results.session_id, "scan-test");

    // Work started after the request is a different session and is not flagged.
    let (_late_id, late_flag) = begin_scan(&state);
    assert!(
        !late_flag.load(Ordering::Relaxed),
        "a later session is unrelated to the close request"
    );
}

#[test]
fn close_request_still_waits_for_an_in_flight_clean_after_cancelling_scans() {
    let state = common::make_state();
    let (_scan_id, scan_flag) = begin_scan(&state);
    let clean_guard = state.clean_gate.enter().expect("in-flight Clean admitted");
    let request_id = request_close(&state);
    assert!(scan_flag.load(Ordering::Relaxed));

    let worker_state = state.clone();
    let worker = thread::spawn(move || {
        worker_state
            .close_gate
            .record_decision(CloseDecision::Approved, &request_id)
    });

    wait_for_clean_gate_sealed(&state);
    assert!(
        state.clean_gate.enter().is_none(),
        "close must not exit while a Clean is in flight"
    );
    drop(clean_guard);
    assert_eq!(worker.join().expect("decision worker"), CloseApproval::Exit);
}
