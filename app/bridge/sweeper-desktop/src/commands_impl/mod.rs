//! Command implementations (headless-testable).

mod analyzer;
mod clean;
mod elevation;
mod scan;
mod settings_logs;
mod shared;
#[cfg(test)]
mod tests;

pub(crate) fn is_sandbox_path_hint(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    const MARKERS: [&str; 4] = [
        "sweeper-sandbox",
        ".sweeper_sandbox_sentinel",
        ".sweeper-sentinel",
        "sweeper-test",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
}

pub use analyzer::{cancel_analyzer, start_analyzer};
pub use clean::{execute_clean, execute_clean_default};
pub use elevation::{get_elevation_status, relaunch_elevated};
pub use scan::{explain_item, get_scan_results, preview_clean, start_scan};
pub use settings_logs::{
    get_settings, list_logs, list_logs_in_dir, read_log, read_log_in_dir, update_settings,
    validate_custom_path,
};
pub use shared::join_error_to_contract_error;
