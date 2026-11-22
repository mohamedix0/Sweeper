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

/// Generation snapshots captured at session start and read again at commit.
/// Both surfaces are carried so each session kind can validate against the
/// generation its validity actually depends on.
#[derive(Clone, Copy, Debug)]
pub(super) struct CommitGenerations {
    pub scan_at_start: u64,
    pub scan_current: u64,
    pub analyzer_at_start: u64,
    pub analyzer_current: u64,
}

/// Generation snapshots captured when an async operation started. Neither is
/// derivable at commit time, so both travel together as one value object.
#[derive(Clone, Copy, Debug)]
pub(super) struct CommitStartGenerations {
    pub scan: u64,
    pub analyzer: u64,
}

pub(super) struct AsyncOutcomeCommit<P, R, F> {
    pub session_id: String,
    pub start_generations: CommitStartGenerations,
    pub payload: P,
    pub finish: F,
    _marker: std::marker::PhantomData<fn() -> R>,
}

impl<P, R, F> AsyncOutcomeCommit<P, R, F> {
    pub fn new(
        session_id: String,
        start_generations: CommitStartGenerations,
        payload: P,
        finish: F,
    ) -> Self {
        Self {
            session_id,
            start_generations,
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
    F: FnOnce(&C, &mut SessionStore, String, CommitGenerations, P) -> Result<R, ContractError>,
{
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let generations = CommitGenerations {
        scan_at_start: req.start_generations.scan,
        scan_current: settings.generation(),
        analyzer_at_start: req.start_generations.analyzer,
        analyzer_current: settings.analyzer_generation(),
    };
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    (req.finish)(
        &state.clock,
        &mut sessions,
        req.session_id,
        generations,
        req.payload,
    )
}

const GENERIC_REASON: &str = "background task failed";
const CANCELLED_REASON: &str = "background task was cancelled";
const PANICKED_REASON: &str = "background task panicked";

/// Already-safe display strings, retained only as a fallback for callers that
/// pass a plain string instead of the typed `tauri::Error` from `spawn_blocking`.
const SAFE_JOIN_DETAILS: [&str; 3] = [
    "worker panicked while unlocking the session store",
    "task panicked",
    "task was cancelled",
];

/// Classify the real Tauri/tokio join error by kind. `JoinError`'s display text
/// embeds a task id and possibly a panic payload, so it is never surfaced (#99).
fn typed_join_reason<E: std::fmt::Display + 'static>(e: &E) -> Option<&'static str> {
    let err = (e as &dyn std::any::Any).downcast_ref::<tauri::Error>()?;
    let tauri::Error::JoinError(join) = err else {
        return None;
    };
    Some(if join.is_cancelled() {
        CANCELLED_REASON
    } else if join.is_panic() {
        PANICKED_REASON
    } else {
        GENERIC_REASON
    })
}

/// Map a blocking-task join error to `ContractError` without leaking internals.
///
/// Real `tauri::Error::JoinError` values are classified by kind into fixed safe
/// reasons; plain strings fall back to the small allowlist of safe shapes (#99).
pub fn join_error_to_contract_error<E: std::fmt::Display + 'static>(e: E) -> ContractError {
    let reason = typed_join_reason(&e)
        .map(str::to_string)
        .unwrap_or_else(|| {
            let detail = e.to_string();
            let detail = detail.trim();
            if SAFE_JOIN_DETAILS.contains(&detail) {
                format!("{GENERIC_REASON}: {detail}")
            } else {
                GENERIC_REASON.to_string()
            }
        });
    ContractError::Internal { reason }
}
