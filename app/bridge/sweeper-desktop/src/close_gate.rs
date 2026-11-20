//! Coordination of window-close requests against an unsaved Settings draft.
//!
//! A Tauri `CloseRequested` handler must never exit while an unsaved draft
//! might still be lost, nor while a `Clean` is in flight. This module holds the
//! authoritative backend handshake, independent of any JavaScript
//! `beforeunload` handler:
//!
//! 1. `begin_close` defers the native close and asks the webview to flush/save
//!    its draft and decide.
//! 2. `record_decision` either keeps the app open (cancel or failed save) or
//!    seals the shared [`CleanGate`], waits for in-flight Cleans, and reports
//!    that the caller may exit.
//!
//! `beforeunload` in the frontend is only a complementary safety net; a close
//! decided here is sufficient on its own.

use std::sync::{Arc, Mutex};

use crate::clean_gate::CleanGate;

/// Internal handshake state. Kept in one lock so a decision cannot race a new
/// close request into a second prompt or an early exit.
#[derive(Debug, Default)]
struct CloseState {
    awaiting: bool,
    exiting: bool,
}

/// What the window-event handler must do with a `CloseRequested` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseRequest {
    /// Prevent the native close and ask the webview to flush/save and decide.
    DeferAndAsk,
    /// A handshake is already pending. The native close is still prevented,
    /// but no second prompt may be started.
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
    /// Remain open: the decision was a cancel, a failed save, or stale.
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
    /// reported as [`CloseRequest::AlreadyPending`].
    pub fn begin_close(&self) -> CloseRequest {
        let mut state = self.lock_state();
        if state.awaiting || state.exiting {
            CloseRequest::AlreadyPending
        } else {
            state.awaiting = true;
            CloseRequest::DeferAndAsk
        }
    }

    /// Record the webview's decision. A decision is honoured only while a
    /// handshake is pending. Approval seals the shared Clean gate and waits for
    /// in-flight Cleans before returning [`CloseApproval::Exit`]; both the seal
    /// and the pending-state transition happen under one lock acquisition.
    pub fn record_decision(&self, decision: CloseDecision) -> CloseApproval {
        {
            let mut state = self.lock_state();
            if !state.awaiting || state.exiting {
                return CloseApproval::StayOpen;
            }
            state.awaiting = false;
            if decision == CloseDecision::Cancelled {
                return CloseApproval::StayOpen;
            }
            state.exiting = true;
        }
        self.clean_gate.seal_and_wait_for_idle();
        CloseApproval::Exit
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, CloseState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
