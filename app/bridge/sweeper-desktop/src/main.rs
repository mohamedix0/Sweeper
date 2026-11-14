#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! Sweeper desktop binary (Tauri 2 shell).

use sweeper_contract::{
    AnalyzerProgressEvent, CleanProgressEvent, ContractError, ElevationStatusDto,
    ExecuteCleanResponse, ExplainItemResponse, LogMetadataDto, LogReadResponse,
    PreviewCleanResponse, PreviewConfirmationsDto, RelaunchResultDto, ScanResultsDto, SettingsDto,
    StartAnalyzerResponse, StartScanResponse, TierDto, ValidateCustomPathResponse,
};
use sweeper_desktop::{commands_impl, AppState, SystemClock};
use tauri::Emitter;

#[tauri::command]
fn start_scan(
    state: tauri::State<'_, AppState<SystemClock>>,
    tiers: Option<Vec<TierDto>>,
) -> Result<StartScanResponse, ContractError> {
    commands_impl::start_scan(&state, tiers)
}

#[tauri::command]
fn get_scan_results(
    state: tauri::State<'_, AppState<SystemClock>>,
    session_id: String,
) -> Result<ScanResultsDto, ContractError> {
    commands_impl::get_scan_results(&state, session_id)
}

#[tauri::command]
fn preview_clean(
    state: tauri::State<'_, AppState<SystemClock>>,
    session_id: String,
    item_ids: Vec<String>,
    confirmations: Option<PreviewConfirmationsDto>,
) -> Result<PreviewCleanResponse, ContractError> {
    commands_impl::preview_clean(&state, session_id, item_ids, confirmations)
}

#[tauri::command]
fn execute_clean(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState<SystemClock>>,
    preview_token: String,
) -> Result<ExecuteCleanResponse, ContractError> {
    commands_impl::execute_clean(&state, preview_token, None, |evt: CleanProgressEvent| {
        let _ = app.emit("CleanProgressEvent", evt);
    })
}

#[tauri::command]
fn start_analyzer(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState<SystemClock>>,
    root: Option<String>,
) -> Result<StartAnalyzerResponse, ContractError> {
    let app_for_progress = app.clone();
    commands_impl::start_analyzer(
        &state,
        root,
        Some(move |evt: AnalyzerProgressEvent| {
            let _ = app_for_progress.emit("AnalyzerProgressEvent", evt);
        }),
    )
}

#[tauri::command]
fn explain_item(
    state: tauri::State<'_, AppState<SystemClock>>,
    item_id: String,
) -> Result<ExplainItemResponse, ContractError> {
    commands_impl::explain_item(&state, item_id)
}

#[tauri::command]
fn get_settings(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<SettingsDto, ContractError> {
    commands_impl::get_settings(&state)
}

#[tauri::command]
fn update_settings(
    state: tauri::State<'_, AppState<SystemClock>>,
    settings: SettingsDto,
) -> Result<SettingsDto, ContractError> {
    commands_impl::update_settings(&state, settings)
}

#[tauri::command]
fn validate_custom_path(
    state: tauri::State<'_, AppState<SystemClock>>,
    path: String,
) -> Result<ValidateCustomPathResponse, ContractError> {
    commands_impl::validate_custom_path(&state, path)
}

#[tauri::command]
fn list_logs(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<Vec<LogMetadataDto>, ContractError> {
    commands_impl::list_logs(&state)
}

#[tauri::command]
fn read_log(
    state: tauri::State<'_, AppState<SystemClock>>,
    log_id: String,
) -> Result<LogReadResponse, ContractError> {
    commands_impl::read_log(&state, log_id)
}

#[tauri::command]
fn get_elevation_status(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<ElevationStatusDto, ContractError> {
    commands_impl::get_elevation_status(&state)
}

#[tauri::command]
fn relaunch_elevated(
    state: tauri::State<'_, AppState<SystemClock>>,
) -> Result<RelaunchResultDto, ContractError> {
    commands_impl::relaunch_elevated(&state)
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
