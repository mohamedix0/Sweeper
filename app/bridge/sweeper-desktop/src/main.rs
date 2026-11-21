#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! Sweeper desktop binary (Tauri 2 shell).

use sweeper_contract::{
    AnalyzerProgressEvent, CleanProgressEvent, CloseReadyResultDto, CloseRequestDto, ContractError,
    ElevationStatusDto, ExecuteCleanResponse, ExplainItemResponse, LogMetadataDto, LogReadResponse,
    PreviewCleanResponse, PreviewConfirmationsDto, RelaunchResultDto, ScanProgressEvent,
    ScanResultsDto, SettingsDto, StartAnalyzerResponse, StartScanResponse, TierDto,
    ValidateCustomPathResponse,
};
use sweeper_desktop::close_gate::{CloseApproval, CloseDecision, CloseRequest};
use sweeper_desktop::{commands_impl, AppState, SystemClock};
use tauri::Emitter;

/// Host-to-webview event asking the app shell to flush/save its draft and decide.
const CLOSE_REQUESTED_EVENT: &str = "CloseRequestedByHost";

fn emit_event<T: serde::Serialize + Clone + Send + 'static>(
    app: &tauri::AppHandle,
    event: &'static str,
    payload: T,
) {
    let _ = app.emit(event, payload);
}

async fn run_blocking<T, F>(
    state: tauri::State<'_, AppState<SystemClock>>,
    f: F,
) -> Result<T, ContractError>
where
    T: Send + 'static,
    F: FnOnce(&AppState<SystemClock>) -> Result<T, ContractError> + Send + 'static,
{
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || f(&state))
        .await
        .map_err(commands_impl::join_error_to_contract_error)?
}

fn event_cb<T>(app: &tauri::AppHandle, event: &'static str) -> impl Fn(T) + Send + Sync + 'static
where
    T: serde::Serialize + Clone + Send + 'static,
{
    let app = app.clone();
    move |payload: T| emit_event(&app, event, payload)
}

macro_rules! run_cmd {
    ($state:expr, $s:ident => $body:expr) => {
        run_blocking($state, move |$s| $body).await
    };
}

#[tauri::command]
async fn start_scan(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState<SystemClock>>,
    tiers: Option<Vec<TierDto>>,
) -> Result<StartScanResponse, ContractError> {
    let on_progress = event_cb::<ScanProgressEvent>(&app, "ScanProgressEvent");
    run_cmd!(state, s => commands_impl::start_scan(s, tiers, Some(on_progress)))
}

#[tauri::command]
async fn get_scan_results(
    state: tauri::State<'_, AppState<SystemClock>>,
    session_id: String,
) -> Result<ScanResultsDto, ContractError> {
    run_cmd!(state, s => commands_impl::get_scan_results(s, session_id))
}

/// Cancel one in-flight Scan by session id only. Never accepts or forwards a
/// path; the backend flags just the named ScanSession (#94).
#[tauri::command]
async fn cancel_scan(
    state: tauri::State<'_, AppState<SystemClock>>,
    session_id: String,
) -> Result<(), ContractError> {
    run_blocking(state, move |s| commands_impl::cancel_scan(s, session_id)).await
}

#[tauri::command]
async fn preview_clean(
    state: tauri::State<'_, AppState<SystemClock>>,
    session_id: String,
    item_ids: Vec<String>,
    confirmations: Option<PreviewConfirmationsDto>,
) -> Result<PreviewCleanResponse, ContractError> {
    run_cmd!(state, s => commands_impl::preview_clean(s, session_id, item_ids, confirmations))
}

#[tauri::command]
async fn execute_clean(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState<SystemClock>>,
    preview_token: String,
) -> Result<ExecuteCleanResponse, ContractError> {
    let on_progress = event_cb::<CleanProgressEvent>(&app, "CleanProgressEvent");
    run_cmd!(state, s => commands_impl::execute_clean(s, preview_token, None, on_progress))
}

#[tauri::command]
async fn start_analyzer(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState<SystemClock>>,
    root: Option<String>,
) -> Result<StartAnalyzerResponse, ContractError> {
    let on_progress = event_cb::<AnalyzerProgressEvent>(&app, "AnalyzerProgressEvent");
    run_cmd!(state, s => commands_impl::start_analyzer(s, root, Some(on_progress)))
}

#[tauri::command]
async fn cancel_analyzer(
    state: tauri::State<'_, AppState<SystemClock>>,
    session_id: String,
) -> Result<(), ContractError> {
    run_blocking(state, move |s| {
        commands_impl::cancel_analyzer(s, session_id)
    })
    .await
}

#[tauri::command]
async fn explain_item(
    state: tauri::State<'_, AppState<SystemClock>>,
    item_id: String,
) -> Result<ExplainItemResponse, ContractError> {
    run_blocking(state, move |s| commands_impl::explain_item(s, item_id)).await
}

#[tauri::command]
async fn get_settings(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<SettingsDto, ContractError> {
    run_blocking(state, commands_impl::get_settings).await
}

#[tauri::command]
async fn update_settings(
    state: tauri::State<'_, AppState<SystemClock>>,
    settings: SettingsDto,
) -> Result<SettingsDto, ContractError> {
    run_blocking(state, move |s| commands_impl::update_settings(s, settings)).await
}

#[tauri::command]
async fn validate_custom_path(
    state: tauri::State<'_, AppState<SystemClock>>,
    path: String,
) -> Result<ValidateCustomPathResponse, ContractError> {
    run_blocking(state, move |s| commands_impl::validate_custom_path(s, path)).await
}

#[tauri::command]
async fn list_logs(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<Vec<LogMetadataDto>, ContractError> {
    run_blocking(state, commands_impl::list_logs).await
}

#[tauri::command]
async fn read_log(
    state: tauri::State<'_, AppState<SystemClock>>,
    log_id: String,
) -> Result<LogReadResponse, ContractError> {
    run_blocking(state, move |s| commands_impl::read_log(s, log_id)).await
}

#[tauri::command]
async fn get_elevation_status(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<ElevationStatusDto, ContractError> {
    run_blocking(state, commands_impl::get_elevation_status).await
}

#[tauri::command]
async fn relaunch_elevated(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<RelaunchResultDto, ContractError> {
    run_blocking(state, commands_impl::relaunch_elevated).await
}

/// Webview's answer to a deferred close request. The request id is echoed back
/// so a late decision for an older request cannot answer a newer one, and a
/// retried decision is idempotent (#93). Sealing the Clean gate and waiting for
/// in-flight Cleans happens on a blocking worker, never on the async runtime or
/// the window-event thread.
#[tauri::command]
async fn resolve_close(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState<SystemClock>>,
    approved: bool,
    request_id: String,
) -> Result<(), ContractError> {
    let decision = if approved {
        CloseDecision::Approved
    } else {
        CloseDecision::Cancelled
    };
    let approval = run_blocking(state, move |s| {
        Ok(s.close_gate.record_decision(decision, &request_id))
    })
    .await?;
    if approval == CloseApproval::Exit {
        app.exit(0);
    }
    Ok(())
}

/// Registration-time pending-state query. When the webview's close listener
/// becomes ready it calls this; any request that arrived while no listener was
/// registered is returned for immediate delivery (never auto-approved) (#93).
#[tauri::command]
async fn notify_close_listener_ready(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<CloseReadyResultDto, ContractError> {
    run_blocking(state, move |s| {
        Ok(CloseReadyResultDto {
            pending_request: s
                .close_gate
                .pending_request()
                .map(|request_id| CloseRequestDto { request_id }),
        })
    })
    .await
}

/// Clear a pending close left by an unreachable listener. Used after bounded
/// listener-registration retries fail, so a later close is a fresh request and
/// cannot be swallowed as `AlreadyPending` (#93). The native close stays
/// prevented until a decision is actually delivered.
#[tauri::command]
async fn reset_pending_close(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<(), ContractError> {
    run_blocking(state, move |s| {
        s.close_gate.reset_pending();
        Ok(())
    })
    .await
}

fn main() {
    let single_instance_guard = match sweeper_desktop::elevation::acquire_startup_single_instance()
    {
        Ok(guard) => guard,
        Err(_) => {
            eprintln!("another Sweeper instance is already running");
            std::process::exit(1);
        }
    };
    let app_state = AppState::new_default().with_single_instance_guard(single_instance_guard);
    // The close hook shares the #93 handshake state with the managed app state;
    // cloning the Arc-backed state keeps one gate, not a second close path.
    let close_state = app_state.clone();
    #[cfg(has_frontend_dist)]
    let context = tauri::generate_context!();
    #[cfg(not(has_frontend_dist))]
    let context = include!(concat!(env!("OUT_DIR"), "/tauri_context.rs"));

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            start_scan,
            get_scan_results,
            cancel_scan,
            preview_clean,
            execute_clean,
            start_analyzer,
            cancel_analyzer,
            explain_item,
            get_settings,
            update_settings,
            validate_custom_path,
            list_logs,
            read_log,
            get_elevation_status,
            relaunch_elevated,
            resolve_close,
            notify_close_listener_ready,
            reset_pending_close
        ])
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Always prevent the native close first: an unsaved draft must
                // never be lost just because the OS asked the window to close.
                api.prevent_close();
                if let CloseRequest::DeferAndAsk { request_id } = close_state.begin_close_request()
                {
                    let _ = window.emit(CLOSE_REQUESTED_EVENT, CloseRequestDto { request_id });
                }
            }
        })
        .run(context)
        .expect("error while running tauri application");
}
