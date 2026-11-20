use std::path::PathBuf;

use sweeper_contract::{CleanProgressEvent, ContractError, ExecuteCleanResponse};

use crate::clock::Clock;
use crate::session::{SessionStore, ValidatedCleanBatch};
use crate::AppState;

use super::shared::lock_state_error;

/// `execute_clean` implementation (test-injected logs dir supported).
///
/// Performs staged locking: atomically validates and consumes `preview_token` under short lock,
/// runs removal without holding `SessionStore`/settings locks.
pub fn execute_clean<C: Clock, F>(
    state: &AppState<C>,
    preview_token: String,
    journal_base_dir: Option<PathBuf>,
    on_progress: F,
) -> Result<ExecuteCleanResponse, ContractError>
where
    F: FnMut(CleanProgressEvent),
{
    if is_e2e_read_only() {
        return Err(ContractError::Internal {
            reason: "Clean is disabled while SWEEPER_E2E_READ_ONLY=1 is active".into(),
        });
    }

    // Hold a gate token for the whole removal so an Elevate-triggered restart
    // waits for this Clean to settle instead of terminating it (#92). If the
    // gate is sealed for shutdown, reject before consuming the token or
    // deleting anything: a new Clean must never start during handoff.
    let _clean_guard = state
        .clean_gate
        .enter()
        .ok_or_else(|| ContractError::Internal {
            reason: "Clean is unavailable while an elevation restart is in progress".into(),
        })?;

    // Atomic validation + consumption of preview token under lock (prevents double execute races).
    let (min_age_hours, validated_items, preview_estimated_bytes) = {
        let settings = state.settings.lock().map_err(|_| lock_state_error())?;
        let settings_gen = settings.generation();
        let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
        sessions.consume_preview_token_for_execute(&state.clock, &preview_token, settings_gen)?
    };

    SessionStore::execute_validated_clean_batch(
        &state.clock,
        ValidatedCleanBatch {
            preview_token,
            min_age_hours,
            validated_items,
            preview_estimated_bytes,
            journal_base_dir,
        },
        on_progress,
    )
}

fn is_e2e_read_only() -> bool {
    std::env::var_os("SWEEPER_E2E_READ_ONLY").as_deref() == Some(std::ffi::OsStr::new("1"))
}

/// `execute_clean` using default journal log location.
pub fn execute_clean_default<C: Clock>(
    state: &AppState<C>,
    preview_token: String,
) -> Result<ExecuteCleanResponse, ContractError> {
    execute_clean(state, preview_token, None, |_| {})
}
