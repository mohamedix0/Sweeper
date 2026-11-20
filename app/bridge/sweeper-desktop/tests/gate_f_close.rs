//! Gate F — window-close coordination, independent of the webview `beforeunload` handler.
//!
//! The Rust `CloseRequested` handshake must be fully exercisable by driving the
//! close gate directly. These tests never start a Tauri runtime or run any
//! JavaScript: the whole state machine (defer, pending query, generation-checked
//! decision, re-arm, recovery, and graceful sealing of in-flight Cleans) lives
//! in `sweeper_desktop::close_gate`.

use std::thread;
use std::time::{Duration, Instant};

use sweeper_desktop::clean_gate::CleanGate;
use sweeper_desktop::close_gate::{CloseApproval, CloseDecision, CloseGate, CloseRequest};

const SEAL_TIMEOUT: Duration = Duration::from_secs(2);

fn gate() -> CloseGate {
    CloseGate::new(CleanGate::default())
}

fn defer(gate: &CloseGate) -> String {
    match gate.begin_close() {
        CloseRequest::DeferAndAsk { request_id } => request_id,
        other => panic!("expected DeferAndAsk, got {other:?}"),
    }
}

// Poll an observable property of the gate instead of sleeping a fixed time. The
// in-flight Clean guard stays alive in the caller, so `enter() == None` can only
// mean the decision worker actually sealed the gate (#93 review finding 4).
fn wait_for_gate_sealed(clean_gate: &CleanGate) {
    let deadline = Instant::now() + SEAL_TIMEOUT;
    while Instant::now() < deadline {
        if clean_gate.enter().is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("clean gate was not sealed within {SEAL_TIMEOUT:?}");
}

#[test]
fn close_request_defers_and_defers_only_once() {
    let gate = gate();
    let first = gate.begin_close();
    assert!(matches!(first, CloseRequest::DeferAndAsk { .. }));
    // A repeated OS close while the handshake is pending must not start a
    // second webview prompt, but the caller still prevents the native close.
    assert_eq!(gate.begin_close(), CloseRequest::AlreadyPending);
}

#[test]
fn approval_exits_after_the_handshake() {
    let gate = gate();
    let request_id = defer(&gate);
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &request_id),
        CloseApproval::Exit,
    );
    // Once exiting, further decisions cannot revive the gate. Replayed approvals
    // are idempotent: they never trigger a second exit (#93).
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &request_id),
        CloseApproval::StayOpen,
    );
    assert_eq!(gate.begin_close(), CloseRequest::AlreadyPending);
}

#[test]
fn cancellation_stays_open_and_rearms_for_the_next_close() {
    let gate = gate();
    let first = defer(&gate);
    assert_eq!(
        gate.record_decision(CloseDecision::Cancelled, &first),
        CloseApproval::StayOpen,
    );
    // A cancel (or a failed save) must leave the app usable: the next real
    // close asks again with a fresh id rather than being silently swallowed.
    let second = defer(&gate);
    assert_ne!(second, first);
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &second),
        CloseApproval::Exit,
    );
}

#[test]
fn a_decision_without_a_pending_request_is_ignored() {
    let gate = gate();
    // A stale or forged decision must never exit a process that never asked.
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, "stale"),
        CloseApproval::StayOpen,
    );
    // The still-pending request is untouched by that stale decision.
    let request_id = defer(&gate);
    assert_eq!(gate.pending_request().as_deref(), Some(request_id.as_str()));
}

#[test]
fn approval_seals_the_gate_and_waits_for_an_in_flight_clean() {
    let clean_gate = CleanGate::default();
    let gate = CloseGate::new(clean_gate.clone());
    let clean_guard = clean_gate
        .enter()
        .expect("in-flight Clean must be admitted");
    let request_id = defer(&gate);

    let worker_gate = gate.clone();
    let worker =
        thread::spawn(move || worker_gate.record_decision(CloseDecision::Approved, &request_id));

    // Bounded wait for the seal; no worker-start signal, no fixed sleep.
    wait_for_gate_sealed(&clean_gate);

    // Letting the in-flight Clean settle releases the waiter, which exits only
    // after every Clean has finished — no Clean is killed mid-run (#92).
    drop(clean_guard);
    assert_eq!(worker.join().expect("decision worker"), CloseApproval::Exit);
    assert!(clean_gate.enter().is_none());
}

#[test]
fn close_approval_seal_survives_a_cancelled_relaunch() {
    let clean_gate = CleanGate::default();
    let gate = CloseGate::new(clean_gate.clone());
    // A relaunch owns a reversible seal while its UAC prompt is pending.
    let relaunch_seal = clean_gate
        .try_begin_relaunch()
        .expect("relaunch claims the single slot");
    let request_id = defer(&gate);

    // Close approval upgrades the seal to the terminal Close owner.
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &request_id),
        CloseApproval::Exit,
    );
    // Cancelling the relaunch must not lift the seal owned by the close.
    drop(relaunch_seal);
    assert!(
        clean_gate.enter().is_none(),
        "close-approval seal must persist until process exit"
    );
    assert_eq!(gate.begin_close(), CloseRequest::AlreadyPending);
}

#[test]
fn pending_request_is_queryable_without_approval() {
    let gate = gate();
    let request_id = defer(&gate);
    // A close that arrived before the listener existed is returned on the
    // listener-ready query, but is never auto-approved (#93).
    assert_eq!(gate.pending_request().as_deref(), Some(request_id.as_str()));
    assert_eq!(gate.begin_close(), CloseRequest::AlreadyPending);
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &request_id),
        CloseApproval::Exit,
    );
}

#[test]
fn reset_pending_rearms_a_stuck_handshake() {
    let gate = gate();
    let first = defer(&gate);
    // Listener registration failed: clear the gate so a later close reaches a
    // registered listener instead of returning AlreadyPending forever (#93).
    gate.reset_pending();
    assert_eq!(gate.pending_request(), None);
    let second = defer(&gate);
    assert_ne!(second, first);
    // A decision for the abandoned request cannot answer the new one.
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &first),
        CloseApproval::StayOpen,
    );
    assert_eq!(gate.pending_request().as_deref(), Some(second.as_str()));
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &second),
        CloseApproval::Exit,
    );
}

#[test]
fn a_stale_generation_cannot_answer_a_newer_request() {
    let gate = gate();
    let first = defer(&gate);
    assert_eq!(
        gate.record_decision(CloseDecision::Cancelled, &first),
        CloseApproval::StayOpen,
    );
    let second = defer(&gate);
    // A late decision for the older request is ignored, leaving the newer one
    // pending and answerable (#93).
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &first),
        CloseApproval::StayOpen,
    );
    assert_eq!(gate.pending_request().as_deref(), Some(second.as_str()));
    assert_eq!(
        gate.record_decision(CloseDecision::Approved, &second),
        CloseApproval::Exit,
    );
}
