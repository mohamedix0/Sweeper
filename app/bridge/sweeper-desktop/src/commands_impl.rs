//! Command implementations (headless-testable).

use std::fs;
use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};
use sweeper_contract::{
    AnalyzerProgressEvent, CleanProgressEvent, ContractError, ElevationStatusDto,
    ExecuteCleanResponse, ExplainItemResponse, LogMetadataDto, LogReadResponse,
    PreviewCleanResponse, PreviewConfirmationsDto, RelaunchResultDto, ScanProgressEvent,
    ScanResultsDto, SettingsDto, StartAnalyzerResponse, StartScanResponse, TierDto, U64String,
    ValidateCustomPathResponse,
};
use sweeper_core::analyzer::{self, AnalyzerProgress};
use sweeper_core::scanner::{self, ScanError, ScanProgress};

use crate::clock::Clock;
use crate::session::SessionStore;
use crate::AppState;

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
    let (settings, settings_gen) = {
        let store = state.settings.lock().map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?;
        (store.core(), store.generation())
    };

    let prepared = {
        let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?;
        sessions.begin_scan(&state.clock, tiers, &settings, settings_gen)?
    };

    let session_id = prepared.session_id.clone();
    let shared_cb: Option<Arc<Mutex<F>>> = on_progress.map(|cb| Arc::new(Mutex::new(cb)));
    let emit = |evt: ScanProgressEvent, shared: &Option<Arc<Mutex<F>>>| {
        if let Some(cb) = shared {
            if let Ok(mut g) = cb.lock() {
                (*g)(evt);
            }
        }
    };
    emit(
        ScanProgressEvent::Started {
            session_id: session_id.clone(),
        },
        &shared_cb,
    );

    let core_cb: Option<scanner::ScanProgressCallback> = shared_cb.as_ref().map(|cb| {
        let sid = session_id.clone();
        let cb = cb.clone();
        Arc::new(move |p: ScanProgress| {
            if let Ok(mut g) = cb.lock() {
                (*g)(ScanProgressEvent::Progress {
                    session_id: sid.clone(),
                    completed_targets: p.completed_targets,
                    total_targets: p.total_targets,
                    current_target_id: p.current_target_id,
                    current_target_name: p.current_target_name,
                    scanned_items: p.scanned_items.to_string(),
                    scanned_bytes: p.scanned_bytes.to_string(),
                });
            }
        }) as scanner::ScanProgressCallback
    });

    let scan_result = scanner::scan_with_settings_current_elevation_with_progress(
        &prepared.effective_settings,
        prepared.cancel.clone(),
        core_cb,
    );

    match scan_result {
        Ok(report) => {
            let current_settings_gen = state
                .settings
                .lock()
                .map_err(|_| ContractError::Internal {
                    reason: "state unavailable".into(),
                })?
                .generation();
            let finish_result = {
                let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
                    reason: "state unavailable".into(),
                })?;
                sessions.finish_scan(
                    &state.clock,
                    &session_id,
                    prepared.settings_generation,
                    current_settings_gen,
                    prepared.effective_settings.min_age_hours,
                    report,
                )
            };
            if let Err(err) = finish_result {
                emit(
                    ScanProgressEvent::Failed {
                        session_id: session_id.clone(),
                        error: err.clone(),
                    },
                    &shared_cb,
                );
                return Err(err);
            }
            emit(
                ScanProgressEvent::Completed {
                    session_id: session_id.clone(),
                },
                &shared_cb,
            );
            Ok(StartScanResponse { session_id })
        }
        Err(ScanError::Cancelled) => {
            if let Ok(mut sessions) = state.sessions.lock() {
                sessions.abort_scan(&session_id);
            }
            emit(
                ScanProgressEvent::Cancelled {
                    session_id: session_id.clone(),
                },
                &shared_cb,
            );
            Err(ContractError::Internal {
                reason: "scan cancelled".to_string(),
            })
        }
        Err(e) => {
            if let Ok(mut sessions) = state.sessions.lock() {
                sessions.abort_scan(&session_id);
            }
            let mapped = ContractError::Internal {
                reason: crate::session::sanitize_reason_pub(&e.to_string()),
            };
            emit(
                ScanProgressEvent::Failed {
                    session_id: session_id.clone(),
                    error: mapped.clone(),
                },
                &shared_cb,
            );
            Err(mapped)
        }
    }
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

/// `start_analyzer` implementation.
///
/// Performs staged locking: read settings/register under short locks, runs analyzer without holding
/// `SessionStore`/settings locks, then commits session under short lock.
pub fn start_analyzer<C, F>(
    state: &AppState<C>,
    root: Option<String>,
    on_progress: Option<F>,
) -> Result<StartAnalyzerResponse, ContractError>
where
    C: Clock,
    F: Fn(AnalyzerProgressEvent) + Send + Sync + 'static,
{
    let (settings, settings_gen) = {
        let s = state.settings.lock().map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?;
        (s.core(), s.generation())
    };

    let prepared = {
        let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?;
        sessions.begin_analyzer(&state.clock, settings_gen)?
    };

    let sid_for_progress = prepared.session_id.clone();
    let progress = on_progress.map(|cb| {
        Arc::new(move |p: AnalyzerProgress| {
            cb(AnalyzerProgressEvent {
                session_id: sid_for_progress.clone(),
                scanned_dirs: U64String(p.scanned_dirs),
                scanned_files: U64String(p.scanned_files),
                scanned_bytes: U64String(p.scanned_bytes),
                current_path: p.current_path,
            });
        }) as Arc<dyn Fn(AnalyzerProgress) + Send + Sync>
    });
    let options = analyzer::AnalyzerOptions {
        allow_unknown: settings.allow_delete_unknown,
        ..analyzer::AnalyzerOptions::default()
    };

    let result =
        match analyzer::analyze_path(root.as_deref(), prepared.cancel.clone(), progress, options) {
            Ok(r) => r,
            Err(e) => {
                if let Ok(mut sessions) = state.sessions.lock() {
                    sessions.abort_analyzer(&prepared.session_id);
                }
                return Err(crate::session::map_analyzer_error_pub(e));
            }
        };

    let current_settings_gen = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .generation();
    let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
        reason: "state unavailable".into(),
    })?;
    sessions.finish_analyzer(
        &state.clock,
        &prepared.session_id,
        prepared.settings_generation,
        current_settings_gen,
        settings.min_age_hours,
        result,
    )
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

/// `update_settings` implementation.
///
/// Bumps settings generation on scan/analyzer/clean-affecting changes and invalidates active sessions.
pub fn update_settings<C: Clock>(
    state: &AppState<C>,
    settings: SettingsDto,
) -> Result<SettingsDto, ContractError> {
    let mut store = state.settings.lock().map_err(|_| ContractError::Internal {
        reason: "state unavailable".into(),
    })?;
    let prev_gen = store.generation();
    let updated = store.update(settings);
    let new_gen = store.generation();
    drop(store);

    if new_gen != prev_gen {
        // Explicitly invalidate active scan/analyzer sessions and preview tokens.
        if let Ok(mut sessions) = state.sessions.lock() {
            sessions.invalidate_all_sessions();
        }
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

/// `execute_clean` implementation (test-injected logs dir supported).
///
/// Performs staged locking: atomically validates and consumes `preview_token` under short lock,
/// runs removal without holding `SessionStore`/settings locks.
pub fn execute_clean<C: Clock, F>(
    state: &AppState<C>,
    preview_token: String,
    journal_base_dir: Option<PathBuf>,
    on_progress: F,
) -> Result<ExecuteCleanResponse, ContractError>
where
    F: FnMut(CleanProgressEvent),
{
    if is_e2e_read_only() {
        return Err(ContractError::Internal {
            reason: "Clean is disabled while SWEEPER_E2E_READ_ONLY=1 is active".into(),
        });
    }

    let settings_gen = state
        .settings
        .lock()
        .map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?
        .generation();

    // Atomic validation + consumption of preview token under lock (prevents double execute races).
    let (min_age_hours, validated_items, preview_estimated_bytes) = {
        let mut sessions = state.sessions.lock().map_err(|_| ContractError::Internal {
            reason: "state unavailable".into(),
        })?;
        sessions.consume_preview_token_for_execute(&state.clock, &preview_token, settings_gen)?
    };

    SessionStore::execute_validated_clean_batch(
        &state.clock,
        &preview_token,
        min_age_hours,
        validated_items,
        preview_estimated_bytes,
        journal_base_dir,
        on_progress,
    )
}

fn is_e2e_read_only() -> bool {
    std::env::var_os("SWEEPER_E2E_READ_ONLY").as_deref() == Some(std::ffi::OsStr::new("1"))
}

/// `execute_clean` using default journal log location.
pub fn execute_clean_default<C: Clock>(
    state: &AppState<C>,
    preview_token: String,
) -> Result<ExecuteCleanResponse, ContractError> {
    execute_clean(state, preview_token, None, |_| {})
}

/// Map a blocking-task join error to `ContractError` without leaking internals.
pub fn join_error_to_contract_error<E>(_e: E) -> ContractError {
    ContractError::Internal {
        reason: "background task failed".into(),
    }
}

/// `get_elevation_status` implementation.
pub fn get_elevation_status<C: Clock>(
    state: &AppState<C>,
) -> Result<ElevationStatusDto, ContractError> {
    crate::elevation::get_elevation_status(state.elevation_provider.as_ref())
}

/// `relaunch_elevated` implementation.
pub fn relaunch_elevated<C: Clock>(
    state: &AppState<C>,
) -> Result<RelaunchResultDto, ContractError> {
    crate::elevation::relaunch_elevated(
        state.elevation_provider.as_ref(),
        state.single_instance.as_ref(),
        state.app_exit.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SystemClock;
    use std::ffi::OsString;
    use std::sync::Mutex;

    static E2E_READ_ONLY_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn execute_clean_rejects_when_e2e_read_only_is_enabled() {
        let _lock = E2E_READ_ONLY_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _guard = EnvVarGuard::set("SWEEPER_E2E_READ_ONLY", "1");
        let state = AppState::<SystemClock>::new_default();

        let err = execute_clean(&state, "preview-token".to_string(), None, |_| {}).unwrap_err();

        assert!(matches!(
            err,
            ContractError::Internal { reason } if reason == "Clean is disabled while SWEEPER_E2E_READ_ONLY=1 is active"
        ));
    }

    #[test]
    fn execute_clean_preserves_normal_behavior_when_e2e_read_only_is_unset() {
        let _lock = E2E_READ_ONLY_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("SWEEPER_E2E_READ_ONLY");
        std::env::remove_var("SWEEPER_E2E_READ_ONLY");
        let state = AppState::<SystemClock>::new_default();

        let err = execute_clean(&state, "no-such-token".to_string(), None, |_| {}).unwrap_err();

        match previous {
            Some(value) => std::env::set_var("SWEEPER_E2E_READ_ONLY", value),
            None => std::env::remove_var("SWEEPER_E2E_READ_ONLY"),
        }

        assert!(matches!(err, ContractError::PreviewTokenInvalid { .. }));
    }
}
