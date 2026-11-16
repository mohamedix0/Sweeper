use std::sync::Arc;

use sweeper_contract::{AnalyzerProgressEvent, ContractError, StartAnalyzerResponse, U64String};
use sweeper_core::analyzer::{self, AnalyzerProgress};

use crate::clock::Clock;
use crate::session::AnalyzerFinishInput;
use crate::AppState;

use super::shared::{commit_async_outcome, lock_state_error};

fn build_analyzer_progress_cb<F>(
    session_id: &str,
    on_progress: Option<F>,
) -> Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>
where
    F: Fn(AnalyzerProgressEvent) + Send + Sync + 'static,
{
    let cb = on_progress?;
    let sid = session_id.to_string();
    Some(Arc::new(move |p: AnalyzerProgress| {
        cb(AnalyzerProgressEvent {
            session_id: sid.clone(),
            scanned_dirs: U64String(p.scanned_dirs),
            scanned_files: U64String(p.scanned_files),
            scanned_bytes: U64String(p.scanned_bytes),
            current_path: p.current_path,
        });
    }))
}

/// `start_analyzer` implementation.
///
/// Performs staged locking: read settings/register under short locks, runs analyzer without holding
/// `SessionStore`/settings locks, then commits session under short lock.
pub fn start_analyzer<C, F>(
    state: &AppState<C>,
    root: Option<String>,
    on_progress: Option<F>,
) -> Result<StartAnalyzerResponse, ContractError>
where
    C: Clock,
    F: Fn(AnalyzerProgressEvent) + Send + Sync + 'static,
{
    let (prepared, settings) = {
        let settings_store = state.settings.lock().map_err(|_| lock_state_error())?;
        let settings = settings_store.core();
        let settings_gen = settings_store.generation();
        let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
        let prepared = sessions.begin_analyzer(&state.clock, settings_gen)?;
        (prepared, settings)
    };
    let progress = build_analyzer_progress_cb(&prepared.session_id, on_progress);
    let options = analyzer::AnalyzerOptions {
        allow_unknown: settings.allow_delete_unknown,
        ..analyzer::AnalyzerOptions::default()
    };
    let result =
        match analyzer::analyze_path(root.as_deref(), prepared.cancel.clone(), progress, options) {
            Ok(r) => r,
            Err(e) => {
                if let Ok(mut sessions) = state.sessions.lock() {
                    sessions.abort_analyzer(&prepared.session_id);
                }
                return Err(crate::session::map_analyzer_error_pub(e));
            }
        };
    commit_analyzer_outcome(state, &prepared, settings.min_age_hours, result)
}

fn commit_analyzer_outcome<C: Clock>(
    state: &AppState<C>,
    prepared: &crate::session::PreparedAnalyzer,
    min_age_hours: u32,
    result: analyzer::AnalyzerResult,
) -> Result<StartAnalyzerResponse, ContractError> {
    commit_async_outcome(
        state,
        prepared.session_id.clone(),
        prepared.settings_generation,
        (min_age_hours, result),
        |clock, sessions, sid, start_gen, current_gen, payload| {
            sessions.finish_analyzer(
                clock,
                AnalyzerFinishInput {
                    session_id: sid,
                    settings_generation_at_start: start_gen,
                    current_settings_generation: current_gen,
                    min_age_hours: payload.0,
                    result: payload.1,
                },
            )
        },
    )
}
