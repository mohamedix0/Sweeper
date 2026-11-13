#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! Sweeper desktop binary (Tauri 2 shell).

use sweeper_contract::{
    ContractError, ExplainItemResponse, LogMetadataDto, LogReadResponse, PreviewCleanResponse,
    PreviewConfirmationsDto, ScanResultsDto, SettingsDto, StartScanResponse, TierDto,
};
use sweeper_desktop::{commands_impl, AppState, SystemClock};

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

fn main() {
    let app_state = AppState::new_default();
    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            start_scan,
            get_scan_results,
            preview_clean,
            explain_item,
            get_settings,
            list_logs,
            read_log
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
