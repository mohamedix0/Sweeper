//! Read-only command implementations (headless-testable).

use std::fs;
use std::path::{Path, PathBuf};

use sweeper_contract::{
    ContractError, ExplainItemResponse, LogMetadataDto, LogReadResponse, PreviewCleanResponse,
    PreviewConfirmationsDto, ScanResultsDto, SettingsDto, StartScanResponse, TierDto, U64String,
};

use crate::clock::Clock;
use crate::AppState;

/// `start_scan` implementation.
pub fn start_scan<C: Clock>(
    state: &AppState<C>,
    tiers: Option<Vec<TierDto>>,
) -> Result<StartScanResponse, ContractError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .get();
    let settings_gen = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .generation();
    let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
        reason: "state unavailable".into(),
    })?;
    sessions.start_scan(&state.clock, tiers, &settings, settings_gen)
}

/// `get_scan_results` implementation.
pub fn get_scan_results<C: Clock>(
    state: &AppState<C>,
    session_id: String,
) -> Result<ScanResultsDto, ContractError> {
    let settings_gen = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .generation();
    let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
        reason: "state unavailable".into(),
    })?;
    sessions.get_scan_results(&state.clock, &session_id, settings_gen)
}

/// `preview_clean` implementation.
pub fn preview_clean<C: Clock>(
    state: &AppState<C>,
    session_id: String,
    item_ids: Vec<String>,
    confirmations: Option<PreviewConfirmationsDto>,
) -> Result<PreviewCleanResponse, ContractError> {
    let settings_gen = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .generation();
    let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
        reason: "state unavailable".into(),
    })?;
    sessions.preview_clean(
        &state.clock,
        &session_id,
        item_ids,
        confirmations,
        settings_gen,
    )
}

/// `explain_item` implementation.
pub fn explain_item<C: Clock>(
    state: &AppState<C>,
    item_id: String,
) -> Result<ExplainItemResponse, ContractError> {
    let settings_gen = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .generation();
    let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
        reason: "state unavailable".into(),
    })?;
    sessions.explain_item(&state.clock, &item_id, settings_gen)
}

/// `get_settings` implementation.
pub fn get_settings<C: Clock>(state: &AppState<C>) -> Result<SettingsDto, ContractError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .get();
    Ok(settings)
}

fn default_logs_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Sweeper").join("logs"))
}

/// `list_logs` implementation.
pub fn list_logs<C: Clock>(_state: &AppState<C>) -> Result<Vec<LogMetadataDto>, ContractError> {
    list_logs_in_dir(default_logs_dir().as_deref())
}

/// `list_logs` in a specific directory (testable).
pub fn list_logs_in_dir(dir: Option<&Path>) -> Result<Vec<LogMetadataDto>, ContractError> {
    let Some(dir) = dir else {
        return Ok(vec![]);
    };
    if !dir.exists() {
        return Ok(vec![]);
    }
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(vec![]),
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if id.is_empty() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let meta = entry.metadata().ok();
        let (modified, size) = meta
            .map(|m| {
                let modified = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| U64String(d.as_secs()));
                let size = Some(U64String(m.len()));
                (modified, size)
            })
            .unwrap_or((None, None));
        out.push(LogMetadataDto {
            id,
            name,
            modified_unix_secs: modified,
            size_bytes: size,
        });
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

/// `read_log` implementation.
pub fn read_log<C: Clock>(
    _state: &AppState<C>,
    log_id: String,
) -> Result<LogReadResponse, ContractError> {
    read_log_in_dir(default_logs_dir().as_deref(), &log_id)
}

/// `read_log` in a specific directory (testable).
pub fn read_log_in_dir(dir: Option<&Path>, log_id: &str) -> Result<LogReadResponse, ContractError> {
    if log_id.contains("..") || log_id.contains('/') || log_id.contains('\\') {
        return Err(ContractError::InvalidInput {
            reason: "invalid log id".into(),
        });
    }
    let Some(dir) = dir else {
        return Err(ContractError::NotFound {
            reason: "log not found".into(),
        });
    };
    // Try common extensions
    let candidates = [
        dir.join(format!("{log_id}.jsonl")),
        dir.join(format!("{log_id}.log")),
        dir.join(log_id),
    ];
    for p in candidates {
        if p.is_file() {
            let content = fs::read_to_string(&p).map_err(|_| ContractError::NotFound {
                reason: "log not found".into(),
            })?;
            return Ok(LogReadResponse {
                id: log_id.to_string(),
                content,
            });
        }
    }
    Err(ContractError::NotFound {
        reason: "log not found".into(),
    })
}
