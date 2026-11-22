//! Command implementations (headless-testable).

mod analyzer;
mod clean;
mod elevation;
mod log_ancestors;
mod scan;
mod settings_logs;
mod shared;
#[cfg(test)]
mod tests;

const SWEEPER_SANDBOX_SENTINEL: &str = ".sweeper_sandbox_sentinel";

pub(crate) fn is_sandbox_path_hint(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return false;
    }
    let p = std::path::Path::new(trimmed);
    has_sentinel_component(p) || has_ancestor_sentinel_file(p)
}

fn has_sentinel_component(path: &std::path::Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(SWEEPER_SANDBOX_SENTINEL)
    })
}

fn has_ancestor_sentinel_file(path: &std::path::Path) -> bool {
    path.ancestors()
        .take(8)
        .any(|ancestor| ancestor.join(SWEEPER_SANDBOX_SENTINEL).exists())
}

pub use analyzer::{cancel_analyzer, start_analyzer};
pub use clean::{execute_clean, execute_clean_default};
pub use elevation::{get_elevation_status, relaunch_elevated};
pub use scan::{cancel_scan, explain_item, get_scan_results, preview_clean, start_scan};
pub(crate) use settings_logs::record_clean_log_summary;
pub use settings_logs::{
    get_settings, list_logs, list_logs_in_dir, read_log, read_log_in_dir, update_settings,
    validate_custom_path,
};
pub use shared::join_error_to_contract_error;
