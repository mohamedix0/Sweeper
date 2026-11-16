use std::sync::{Arc, Mutex};

use sweeper_contract::{
    ContractError, ExplainItemResponse, PreviewCleanResponse, PreviewConfirmationsDto,
    ScanProgressEvent, ScanResultsDto, StartScanResponse, TierDto,
};
use sweeper_core::scanner::{self, ScanError};

use crate::clock::Clock;
use crate::session::ScanFinishInput;
use crate::AppState;

use super::shared::{
    build_scan_progress_cb, commit_async_outcome, emit_scan_event, lock_state_error, SharedScanCb,
};

#[derive(Clone, Copy)]
struct ScanRunCtx<'a, F>
where
    F: FnMut(ScanProgressEvent),
{
    session_id: &'a str,
    prepared: &'a crate::session::PreparedScan,
    shared_cb: &'a Option<SharedScanCb<F>>,
}

fn commit_scan_outcome<C: Clock>(
    state: &AppState<C>,
    session_id: &str,
    prepared: &crate::session::PreparedScan,
    report: scanner::ScanReport,
) -> Result<(), ContractError> {
    commit_async_outcome(
        state,
        session_id.to_string(),
        prepared.settings_generation,
        (prepared.effective_settings.min_age_hours, report),
        |clock, sessions, sid, start_gen, current_gen, payload| {
            sessions.finish_scan(
                clock,
                ScanFinishInput {
                    session_id: sid,
                    settings_generation_at_start: start_gen,
                    current_settings_generation: current_gen,
                    effective_min_age_hours: payload.0,
                    report: payload.1,
                },
            )
        },
    )
}

fn handle_scan_failure<C: Clock, F>(
    state: &AppState<C>,
    ctx: ScanRunCtx<'_, F>,
    err: ContractError,
) -> ContractError
where
    F: FnMut(ScanProgressEvent),
{
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.abort_scan(ctx.session_id);
    }
    emit_scan_event(
        ctx.shared_cb,
        ScanProgressEvent::Failed {
            session_id: ctx.session_id.to_string(),
            error: err.clone(),
        },
    );
    err
}

/// `start_scan` implementation.
///
/// Performs staged locking: read settings and register/pre-allocate session under short locks,
/// runs scan without holding `SessionStore`/settings locks, then commits session under short lock.
/// Emits `ScanProgressEvent` (`Started`, `Progress`, `Cancelled|Completed|Failed`) when `on_progress` is provided.
pub fn start_scan<C, F>(
    state: &AppState<C>,
    tiers: Option<Vec<TierDto>>,
    on_progress: Option<F>,
) -> Result<StartScanResponse, ContractError>
where
    C: Clock,
    F: FnMut(ScanProgressEvent) + Send + 'static,
{
    let prepared = {
        let settings_store = state.settings.lock().map_err(|_| lock_state_error())?;
        let settings = settings_store.core();
        let settings_gen = settings_store.generation();
        let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
        sessions.begin_scan(
            &state.clock,
            crate::session::scan::ScanBeginRequest {
                tiers,
                settings,
                settings_generation: settings_gen,
            },
        )?
    };
    let session_id = prepared.session_id.clone();
    let shared_cb = on_progress.map(|cb| Arc::new(Mutex::new(cb)));
    emit_scan_event(
        &shared_cb,
        ScanProgressEvent::Started {
            session_id: session_id.clone(),
        },
    );
    let core_cb = build_scan_progress_cb(&session_id, &shared_cb);
    let scan_result = scanner::scan_with_settings_current_elevation_with_progress(
        &prepared.effective_settings,
        scanner::ScanRunControl {
            cancel: prepared.cancel.clone(),
            progress_cb: core_cb,
        },
    );
    let ctx = ScanRunCtx {
        session_id: &session_id,
        prepared: &prepared,
        shared_cb: &shared_cb,
    };
    finalize_scan(state, ctx, scan_result)
}

fn finalize_scan<C, F>(
    state: &AppState<C>,
    ctx: ScanRunCtx<'_, F>,
    scan_result: Result<scanner::ScanReport, ScanError>,
) -> Result<StartScanResponse, ContractError>
where
    C: Clock,
    F: FnMut(ScanProgressEvent),
{
    match scan_result {
        Ok(report) => {
            if let Err(err) = commit_scan_outcome(state, ctx.session_id, ctx.prepared, report) {
                return Err(handle_scan_failure(state, ctx, err));
            }
            emit_scan_event(
                ctx.shared_cb,
                ScanProgressEvent::Completed {
                    session_id: ctx.session_id.to_string(),
                },
            );
            Ok(StartScanResponse {
                session_id: ctx.session_id.to_string(),
            })
        }
        Err(ScanError::Cancelled) => Err(handle_scan_cancelled(state, ctx)),
        Err(e) => Err(handle_scan_failure(
            state,
            ctx,
            ContractError::Internal {
                reason: crate::session::sanitize_reason_pub(crate::session::MsgRef(&e.to_string())),
            },
        )),
    }
}

fn handle_scan_cancelled<C: Clock, F>(state: &AppState<C>, ctx: ScanRunCtx<'_, F>) -> ContractError
where
    F: FnMut(ScanProgressEvent),
{
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.abort_scan(ctx.session_id);
    }
    emit_scan_event(
        ctx.shared_cb,
        ScanProgressEvent::Cancelled {
            session_id: ctx.session_id.to_string(),
        },
    );
    ContractError::Internal {
        reason: "scan cancelled".to_string(),
    }
}

/// `get_scan_results` implementation.
pub fn get_scan_results<C: Clock>(
    state: &AppState<C>,
    session_id: String,
) -> Result<ScanResultsDto, ContractError> {
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.get_scan_results(&state.clock, &session_id, settings_gen)
}

/// `preview_clean` implementation.
pub fn preview_clean<C: Clock>(
    state: &AppState<C>,
    session_id: String,
    item_ids: Vec<String>,
    confirmations: Option<PreviewConfirmationsDto>,
) -> Result<PreviewCleanResponse, ContractError> {
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.preview_clean(
        &state.clock,
        crate::session::PreviewCleanRequest {
            session_id,
            item_ids,
            confirmations,
            current_settings_generation: settings_gen,
        },
    )
}

/// `explain_item` implementation.
pub fn explain_item<C: Clock>(
    state: &AppState<C>,
    item_id: String,
) -> Result<ExplainItemResponse, ContractError> {
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.explain_item(&state.clock, &item_id, settings_gen)
}
