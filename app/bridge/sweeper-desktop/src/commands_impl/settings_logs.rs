use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use sweeper_contract::{
    ContractError, LogMetadataDto, LogReadResponse, SettingsDto, U64String,
    ValidateCustomPathResponse,
};

use crate::clock::Clock;
use crate::AppState;

use super::shared::lock_state_error;

/// `get_settings` implementation.
pub fn get_settings<C: Clock>(state: &AppState<C>) -> Result<SettingsDto, ContractError> {
    let settings = state.settings.lock().map_err(|_| lock_state_error())?.get();
    Ok(settings)
}

/// `update_settings` implementation.
///
/// Bumps settings generation on scan/analyzer/clean-affecting changes and invalidates active sessions.
pub fn update_settings<C: Clock>(
    state: &AppState<C>,
    settings: SettingsDto,
) -> Result<SettingsDto, ContractError> {
    let mut store = state.settings.lock().map_err(|_| lock_state_error())?;
    let prev_gen = store.generation();
    let updated = store.update(settings);
    let new_gen = store.generation();

    if new_gen != prev_gen {
        // Explicitly invalidate active scan/analyzer sessions and preview tokens.
        let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
        sessions.invalidate_all_sessions();
    }
    Ok(updated)
}

/// `validate_custom_path` implementation.
pub fn validate_custom_path<C: Clock>(
    _state: &AppState<C>,
    path: String,
) -> Result<ValidateCustomPathResponse, ContractError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ContractError::InvalidInput {
            reason: "path is required".into(),
        });
    }
    let verdict = sweeper_core::policy::validate_custom_path(trimmed);
    let normalized = if verdict.allowed {
        sweeper_core::paths::CanonicalPath::from_existing_path(trimmed)
            .map(|p| p.canonical().to_string())
            .or_else(|_| {
                sweeper_core::paths::CanonicalPath::parse_lexical(trimmed).map(|p| p.to_string())
            })
            .unwrap_or_else(|_| trimmed.to_string())
    } else {
        sweeper_core::paths::CanonicalPath::parse_lexical(trimmed)
            .map(|p| p.to_string())
            .unwrap_or_else(|_| trimmed.to_string())
    };
    Ok(ValidateCustomPathResponse {
        path: normalized,
        allowed: verdict.allowed,
        reason: verdict.reason,
    })
}

fn default_logs_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Sweeper").join("logs"))
}

/// `list_logs` implementation.
pub fn list_logs<C: Clock>(_state: &AppState<C>) -> Result<Vec<LogMetadataDto>, ContractError> {
    list_logs_in_dir_impl(default_logs_dir().as_deref(), true)
}

fn clean_log_phantom_reasons(path: &Path) -> (bool, bool) {
    let Ok(file) = fs::File::open(path) else {
        return (true, false);
    };
    let reader = BufReader::new(file);
    let mut saw_outcome = false;
    let mut saw_path = false;
    let mut all_sandbox = true;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let kind = value.get("kind").and_then(|v| v.as_str()).or_else(|| {
            value
                .get("record")
                .and_then(|v| v.as_str())
                .and_then(|r| match r {
                    "outcome" => Some("outcome"),
                    "intent" => Some("intent"),
                    _ => None,
                })
        });
        if kind == Some("outcome") {
            saw_outcome = true;
        }
        if let Some(p) = value.get("path").and_then(|v| v.as_str()) {
            saw_path = true;
            if !super::is_sandbox_path_hint(p) {
                all_sandbox = false;
            }
        }
        if let Some(target) = value.get("target_path").and_then(|v| v.as_str()) {
            saw_path = true;
            if !super::is_sandbox_path_hint(target) {
                all_sandbox = false;
            }
        }
    }

    let no_outcome = !saw_outcome;
    let sandbox_only = saw_path && all_sandbox;
    (no_outcome, sandbox_only)
}

fn should_include_log_file(path: &Path, size: u64, filter_sandbox_phantoms: bool) -> bool {
    if size == 0 {
        return false;
    }
    let is_clean = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| {
            s.to_ascii_lowercase().starts_with("clean-")
                && s.to_ascii_lowercase().ends_with(".jsonl")
        })
        .unwrap_or(false);
    if is_clean {
        let (no_outcome, sandbox_only) = clean_log_phantom_reasons(path);
        if no_outcome {
            return false;
        }
        if filter_sandbox_phantoms && sandbox_only {
            return false;
        }
    }
    true
}

fn list_logs_in_dir_impl(
    dir: Option<&Path>,
    filter_sandbox_phantoms: bool,
) -> Result<Vec<LogMetadataDto>, ContractError> {
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
        let size_u64 = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        if !should_include_log_file(&path, size_u64, filter_sandbox_phantoms) {
            continue;
        }
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

/// `list_logs` in a specific directory (testable, does not filter sandbox-only clean logs).
pub fn list_logs_in_dir(dir: Option<&Path>) -> Result<Vec<LogMetadataDto>, ContractError> {
    list_logs_in_dir_impl(dir, false)
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
    if log_id.contains("..") {
        return Err(ContractError::InvalidInput {
            reason: "invalid log id".into(),
        });
    }
    if log_id.contains('/') {
        return Err(ContractError::InvalidInput {
            reason: "invalid log id".into(),
        });
    }
    if log_id.contains('\\') {
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
