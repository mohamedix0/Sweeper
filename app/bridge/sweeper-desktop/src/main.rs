#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! Sweeper desktop binary (Tauri 2 shell).

use sweeper_contract::{
    AnalyzerProgressEvent, CleanProgressEvent, ContractError, ElevationStatusDto,
    ExecuteCleanResponse, ExplainItemResponse, LogMetadataDto, LogReadResponse,
    PreviewCleanResponse, PreviewConfirmationsDto, RelaunchResultDto, ScanProgressEvent,
    ScanResultsDto, SettingsDto, StartAnalyzerResponse, StartScanResponse, TierDto,
    ValidateCustomPathResponse,
};
use sweeper_desktop::{commands_impl, AppState, SystemClock};
use tauri::Emitter;

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
    #[cfg(has_frontend_dist)]
    let context = tauri::generate_context!();
    #[cfg(not(has_frontend_dist))]
    let context = include!(concat!(env!("OUT_DIR"), "/tauri_context.rs"));

    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            start_scan,
            get_scan_results,
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
            relaunch_elevated
        ])
        .run(context)
        .expect("error while running tauri application");
}
