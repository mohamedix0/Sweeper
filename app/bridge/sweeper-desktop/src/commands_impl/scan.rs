use std::path::PathBuf;
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
    build_scan_progress_cb, commit_async_outcome, emit_scan_event, lock_state_error,
    AsyncOutcomeCommit, SharedScanCb,
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
        AsyncOutcomeCommit::new(
            session_id.to_string(),
            prepared.settings_generation,
            (prepared.effective_settings.min_age_hours, report),
            |clock: &C,
             sessions: &mut crate::session::SessionStore,
             sid: String,
             start_gen: u64,
             current_gen: u64,
             payload: (u32, scanner::ScanReport)| {
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
        ),
    )
}

fn has_any_builtin_tier_enabled(settings: &sweeper_core::settings::SweeperSettings) -> bool {
    if settings.enable_tier1 || settings.enable_tier2 {
        return true;
    }
    settings.enable_tier3
}

fn custom_path_is_verified_sandbox(path: &str) -> bool {
    if super::is_sandbox_path_hint(path) {
        return true;
    }
    std::path::Path::new(path)
        .join(".sweeper_sandbox_sentinel")
        .exists()
}

fn path_is_within_verified_custom_sandbox(
    settings: &sweeper_core::settings::SweeperSettings,
    path: &str,
) -> bool {
    let candidate = std::path::Path::new(path);
    settings.custom_paths.iter().any(|custom| {
        if !custom_path_is_verified_sandbox(custom) {
            return false;
        }
        candidate.starts_with(custom)
    })
}

fn all_custom_paths_are_sandbox(settings: &sweeper_core::settings::SweeperSettings) -> bool {
    settings
        .custom_paths
        .iter()
        .all(|p| custom_path_is_verified_sandbox(p))
}

fn report_targets_are_sandbox_custom(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
) -> bool {
    report.targets.iter().all(|t| {
        if !t.target_id.starts_with("custom-path") {
            return false;
        }
        t.resolved_paths.iter().all(|p| {
            super::is_sandbox_path_hint(p) || path_is_within_verified_custom_sandbox(settings, p)
        })
    })
}

fn is_sandbox_scan(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
) -> bool {
    if has_any_builtin_tier_enabled(settings) {
        return false;
    }
    if settings.custom_paths.is_empty() {
        return false;
    }
    if !all_custom_paths_are_sandbox(settings) {
        return false;
    }
    report_targets_are_sandbox_custom(settings, report)
}

fn scan_logs_base_dir() -> PathBuf {
    std::env::var_os("SWEEPER_LOGS_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(sweeper_core::journal::default_logs_base_dir)
}

fn write_scan_activity_log(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
) {
    if is_sandbox_scan(settings, report) {
        return;
    }
    let total_reclaimable_bytes: u64 = report.targets.iter().map(|t| t.eligible_bytes).sum();
    let total_eligible_files: u64 = report.targets.iter().map(|t| t.eligible_count).sum();
    let total_skipped_files: u64 = report.targets.iter().map(|t| t.skipped_count).sum();
    let categories: Vec<serde_json::Value> = report
        .targets
        .iter()
        .filter(|t| t.eligible_count > 0 || t.file_count > 0)
        .map(|t| {
            serde_json::json!({
                "target_id": t.target_id,
                "target_name": t.target_name,
                "tier": format!("{:?}", t.tier),
                "risk": format!("{:?}", t.risk),
                "eligible_bytes": t.eligible_bytes.to_string(),
                "total_bytes": t.total_bytes.to_string(),
                "eligible_count": t.eligible_count.to_string(),
                "file_count": t.file_count.to_string(),
                "skipped_count": t.skipped_count.to_string(),
            })
        })
        .collect();
    let record = serde_json::json!({
        "kind": "scan",
        "record": "outcome",
        "event": "scan",
        "timestamp_unix_secs": report.started_unix_secs,
        "total_reclaimable_bytes": total_reclaimable_bytes.to_string(),
        "eligible_bytes": total_reclaimable_bytes.to_string(),
        "total_eligible_files": total_eligible_files.to_string(),
        "total_skipped_files": total_skipped_files.to_string(),
        "categories_count": categories.len(),
        "targets": categories,
    });
    match sweeper_core::journal::write_scan_log(&scan_logs_base_dir(), &record) {
        Ok(outcome) => {
            for warning in outcome.retirement_warnings {
                eprintln!("scan activity log retirement warning: {warning}");
            }
        }
        Err(err) => eprintln!("failed to write scan activity log: {err}"),
    }
}

enum ScanTerminal {
    Cancelled,
    Failed(ContractError),
}

fn handle_scan_terminal<C: Clock, F>(
    state: &AppState<C>,
    ctx: ScanRunCtx<'_, F>,
    outcome: ScanTerminal,
) -> ContractError
where
    F: FnMut(ScanProgressEvent),
{
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.abort_scan(ctx.session_id);
    }
    let (event, err) = match outcome {
        ScanTerminal::Cancelled => (
            ScanProgressEvent::Cancelled {
                session_id: ctx.session_id.to_string(),
            },
            ContractError::Internal {
                reason: "scan cancelled".to_string(),
            },
        ),
        ScanTerminal::Failed(err) => (
            ScanProgressEvent::Failed {
                session_id: ctx.session_id.to_string(),
                error: err.clone(),
            },
            err,
        ),
    };
    emit_scan_event(ctx.shared_cb, event);
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
    let scan_result = scanner::scan_with_settings_options_current_elevation_with_progress(
        &prepared.effective_settings,
        scanner::ScanSettingsOptions::default(),
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
            let log_report = report.clone();
            if let Err(err) = commit_scan_outcome(state, ctx.session_id, ctx.prepared, report) {
                return Err(handle_scan_terminal(state, ctx, ScanTerminal::Failed(err)));
            }
            write_scan_activity_log(&ctx.prepared.effective_settings, &log_report);
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
        Err(ScanError::Cancelled) => Err(handle_scan_terminal(state, ctx, ScanTerminal::Cancelled)),
        Err(e) => Err(handle_scan_terminal(
            state,
            ctx,
            ScanTerminal::Failed(ContractError::Internal {
                reason: crate::session::sanitize_reason_pub(crate::session::MsgRef(&e.to_string())),
            }),
        )),
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
    let item_ids = crate::session::compact_selection::expand_selection_ids(&item_ids);
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
