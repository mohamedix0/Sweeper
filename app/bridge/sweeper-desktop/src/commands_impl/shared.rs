use std::sync::{Arc, Mutex};

use sweeper_contract::{ContractError, ScanProgressEvent};
use sweeper_core::scanner::{self, ScanProgress};

use crate::clock::Clock;
use crate::session::SessionStore;
use crate::AppState;

pub(super) type SharedScanCb<F> = Arc<Mutex<F>>;

pub(super) fn lock_state_error() -> ContractError {
    ContractError::Internal {
        reason: "state unavailable".into(),
    }
}

pub(super) fn emit_scan_event<F>(shared: &Option<SharedScanCb<F>>, evt: ScanProgressEvent)
where
    F: FnMut(ScanProgressEvent),
{
    if let Some(cb) = shared {
        if let Ok(mut g) = cb.lock() {
            (*g)(evt);
        }
    }
}

pub(super) fn build_scan_progress_cb<F>(
    session_id: &str,
    shared: &Option<SharedScanCb<F>>,
) -> Option<scanner::ScanProgressCallback>
where
    F: FnMut(ScanProgressEvent) + Send + 'static,
{
    let cb = shared.as_ref()?.clone();
    let sid = session_id.to_string();
    Some(Arc::new(move |p: ScanProgress| {
        if let Ok(mut g) = cb.lock() {
            (*g)(ScanProgressEvent::Progress {
                session_id: sid.clone(),
                completed_targets: p.completed_targets,
                total_targets: p.total_targets,
                current_target_id: p.current_target_id,
                current_target_name: p.current_target_name,
                scanned_items: p.scanned_items.to_string(),
                scanned_bytes: p.scanned_bytes.to_string(),
            });
        }
    }) as scanner::ScanProgressCallback)
}

pub(super) struct AsyncOutcomeCommit<P, R, F> {
    pub session_id: String,
    pub settings_generation_at_start: u64,
    pub payload: P,
    pub finish: F,
    _marker: std::marker::PhantomData<fn() -> R>,
}

impl<P, R, F> AsyncOutcomeCommit<P, R, F> {
    pub fn new(
        session_id: String,
        settings_generation_at_start: u64,
        payload: P,
        finish: F,
    ) -> Self {
        Self {
            session_id,
            settings_generation_at_start,
            payload,
            finish,
            _marker: std::marker::PhantomData,
        }
    }
}

pub(super) fn commit_async_outcome<C, P, R, F>(
    state: &AppState<C>,
    req: AsyncOutcomeCommit<P, R, F>,
) -> Result<R, ContractError>
where
    C: Clock,
    F: FnOnce(&C, &mut SessionStore, String, u64, u64, P) -> Result<R, ContractError>,
{
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let current_settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    (req.finish)(
        &state.clock,
        &mut sessions,
        req.session_id,
        req.settings_generation_at_start,
        current_settings_gen,
        req.payload,
    )
}

/// Map a blocking-task join error to `ContractError` without leaking internals.
pub fn join_error_to_contract_error<E>(_e: E) -> ContractError {
    ContractError::Internal {
        reason: "background task failed".into(),
    }
}
