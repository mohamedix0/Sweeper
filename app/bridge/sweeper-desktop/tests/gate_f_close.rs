//! Gate F — window-close coordination, independent of the webview `beforeunload` handler.
//!
//! The Rust `CloseRequested` handshake must be fully exercisable by driving the
//! close gate directly. These tests never start a Tauri runtime or run any
//! JavaScript: the whole state machine (defer, approve, cancel, re-arm, and
//! graceful sealing of in-flight Cleans) lives in `sweeper_desktop::close_gate`.

use std::thread;
use std::time::Duration;

use sweeper_desktop::clean_gate::CleanGate;
use sweeper_desktop::close_gate::{CloseApproval, CloseDecision, CloseGate, CloseRequest};

fn gate() -> CloseGate {
    CloseGate::new(CleanGate::default())
}

#[test]
fn close_request_defers_and_defers_only_once() {
    let gate = gate();
    assert_eq!(gate.begin_close(), CloseRequest::DeferAndAsk);
    // A repeated OS close while the handshake is pending must not start a
    // second webview prompt, but the caller still prevents the native close.
    assert_eq!(gate.begin_close(), CloseRequest::AlreadyPending);
}

#[test]
fn approval_exits_after_the_handshake() {
    let gate = gate();
    assert_eq!(gate.begin_close(), CloseRequest::DeferAndAsk);
    assert_eq!(
        gate.record_decision(CloseDecision::Approved),
        CloseApproval::Exit,
    );
    // Once exiting, further decisions cannot revive the gate.
    assert_eq!(
        gate.record_decision(CloseDecision::Approved),
        CloseApproval::StayOpen,
    );
}

#[test]
fn cancellation_stays_open_and_rearms_for_the_next_close() {
    let gate = gate();
    assert_eq!(gate.begin_close(), CloseRequest::DeferAndAsk);
    assert_eq!(
        gate.record_decision(CloseDecision::Cancelled),
        CloseApproval::StayOpen,
    );
    // A cancel (or a failed save) must leave the app usable: the next real
    // close asks again rather than being silently swallowed.
    assert_eq!(gate.begin_close(), CloseRequest::DeferAndAsk);
    assert_eq!(
        gate.record_decision(CloseDecision::Approved),
        CloseApproval::Exit,
    );
}

#[test]
fn a_decision_without_a_pending_request_is_ignored() {
    let gate = gate();
    // A stale or forged decision must never exit a process that never asked.
    assert_eq!(
        gate.record_decision(CloseDecision::Approved),
        CloseApproval::StayOpen,
    );
    assert_eq!(gate.begin_close(), CloseRequest::DeferAndAsk);
}

#[test]
fn approval_seals_the_gate_and_waits_for_an_in_flight_clean() {
    let clean_gate = CleanGate::default();
    let gate = CloseGate::new(clean_gate.clone());
    let clean_guard = clean_gate
        .enter()
        .expect("in-flight Clean must be admitted");
    assert_eq!(gate.begin_close(), CloseRequest::DeferAndAsk);

    let worker_gate = gate.clone();
    let worker = thread::spawn(move || worker_gate.record_decision(CloseDecision::Approved));

    // Give the decision worker time to seal the gate while the Clean is active.
    thread::sleep(Duration::from_millis(50));
    // The seal must block a new Clean and the approval must still be waiting.
    assert!(clean_gate.enter().is_none());

    // Letting the in-flight Clean settle releases the waiter, which exits only
    // after every Clean has finished — no Clean is killed mid-run (#92).
    drop(clean_guard);
    assert_eq!(worker.join().expect("decision worker"), CloseApproval::Exit);
    assert!(clean_gate.enter().is_none());
}
