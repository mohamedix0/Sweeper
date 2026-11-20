//! Coordination of window-close requests against an unsaved Settings draft.
//!
//! A Tauri `CloseRequested` handler must never exit while an unsaved draft
//! might still be lost, nor while a `Clean` is in flight. This module holds the
//! authoritative backend handshake, independent of any JavaScript
//! `beforeunload` handler:
//!
//! 1. `begin_close` defers the native close, assigns a fresh request id, and
//!    asks the webview to flush/save its draft and decide.
//! 2. The webview announces listener readiness with `pending_request`. A close
//!    that arrived before the listener existed is delivered then, never lost
//!    and never auto-approved (#93).
//! 3. `record_decision` honours a decision only while a handshake is pending
//!    **and** its request id matches. A late decision for an older request can
//!    never answer a newer one, and retries are idempotent (#93).
//! 4. Approval seals the shared [`CleanGate`] with a terminal `Close` seal,
//!    waits for in-flight Cleans, and reports that the caller may exit.
//! 5. `reset_pending` clears a pending request left by an unreachable listener
//!    so a later close is a fresh request rather than a swallowed
//!    `AlreadyPending` (#93).
//!
//! `beforeunload` in the frontend is only a complementary safety net; a close
//! decided here is sufficient on its own.

use std::sync::{Arc, Mutex};

use crate::clean_gate::CleanGate;

/// Internal handshake state. Kept in one lock so a decision cannot race a new
/// close request into a second prompt or an early exit.
#[derive(Debug, Default)]
struct CloseState {
    /// The generation id of the request currently awaiting a webview decision.
    pending: Option<String>,
    /// Set once an approval has been recorded; the process is on its way out.
    exiting: bool,
    /// Monotonic source for request ids. Never reused, not even after a reset.
    next_generation: u64,
}

/// What the window-event handler must do with a `CloseRequested` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseRequest {
    /// Prevent the native close and ask the webview to flush/save and decide.
    DeferAndAsk {
        /// The id the webview must echo with its decision.
        request_id: String,
    },
    /// A handshake is already pending or the app is exiting. The native close
    /// is still prevented, but no second prompt may be started.
    AlreadyPending,
}

/// The webview's answer to a pending close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// The draft is clean or was saved; exiting cannot lose an edit.
    Approved,
    /// The user cancelled or the save failed; the app stays open with its draft.
    Cancelled,
}

/// What the caller must do with a recorded decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseApproval {
    /// The gate is sealed, all in-flight Cleans have settled, and the process
    /// may exit. No new Clean can start from this point.
    Exit,
    /// Remain open: the decision was a cancel, stale, or no request was pending.
    StayOpen,
}

/// Tracks one in-flight close handshake and shares the graceful-exit gate.
#[derive(Debug, Clone)]
pub struct CloseGate {
    state: Arc<Mutex<CloseState>>,
    clean_gate: CleanGate,
}

impl CloseGate {
    /// Create a close gate that seals the given shared Clean gate on exit.
    pub fn new(clean_gate: CleanGate) -> Self {
        Self {
            state: Arc::new(Mutex::new(CloseState::default())),
            clean_gate,
        }
    }

    /// Begin a close handshake. Returns [`CloseRequest::DeferAndAsk`] exactly
    /// once per handshake; repeated OS close events while it is pending are
    /// reported as [`CloseRequest::AlreadyPending`]. Each fresh handshake gets
    /// a new id so an older decision cannot answer it (#93).
    pub fn begin_close(&self) -> CloseRequest {
        let mut state = self.lock_state();
        if state.exiting || state.pending.is_some() {
            return CloseRequest::AlreadyPending;
        }
        state.next_generation = state.next_generation.wrapping_add(1);
        let request_id = state.next_generation.to_string();
        state.pending = Some(request_id.clone());
        CloseRequest::DeferAndAsk { request_id }
    }

    /// The pending request a freshly registered listener must deliver. This is
    /// the registration-time pending-state query: the request is returned,
    /// never approved, so a dirty draft still reaches the user's prompt (#93).
    pub fn pending_request(&self) -> Option<String> {
        self.lock_state().pending.clone()
    }

    /// Clear a pending request whose webview listener never became reachable.
    /// A later OS close is then a fresh `DeferAndAsk` instead of being silently
    /// swallowed as `AlreadyPending`. Approval is never implied (#93).
    pub fn reset_pending(&self) {
        let mut state = self.lock_state();
        if !state.exiting {
            state.pending = None;
        }
    }

    /// Record the webview's decision for `request_id`. A decision is honoured
    /// only while that exact request is still pending. Approval seals the
    /// shared Clean gate and waits for in-flight Cleans before returning
    /// [`CloseApproval::Exit`]; both the seal and the pending transition happen
    /// under one lock acquisition so no Clean can slip through.
    pub fn record_decision(&self, decision: CloseDecision, request_id: &str) -> CloseApproval {
        {
            let mut state = self.lock_state();
            if state.exiting || state.pending.as_deref() != Some(request_id) {
                return CloseApproval::StayOpen;
            }
            state.pending = None;
            if decision == CloseDecision::Cancelled {
                return CloseApproval::StayOpen;
            }
            state.exiting = true;
        }
        self.clean_gate.seal_for_close_and_wait();
        CloseApproval::Exit
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, CloseState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
