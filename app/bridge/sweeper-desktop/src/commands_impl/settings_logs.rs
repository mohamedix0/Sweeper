use std::collections::HashSet;
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

#[derive(Default)]
struct CleanLogScanState {
    saw_outcome: bool,
    saw_path: bool,
    all_sandbox: bool,
    /// (target_id, path) pairs for intent records.
    intents: Vec<(String, String)>,
    /// (target_id, path) pairs for outcome records.
    outcomes: HashSet<(String, String)>,
}

impl CleanLogScanState {
    fn sandbox_only(&self) -> bool {
        self.saw_path && self.all_sandbox
    }

    /// Intents with no matching outcome are "unknown, verify on disk": the item
    /// may have been deleted even though its outcome record was never written.
    fn intent_without_outcome(&self) -> usize {
        self.intents
            .iter()
            .filter(|key| !self.outcomes.contains(key))
            .count()
    }
}

fn json_record_kind(value: &serde_json::Value) -> Option<&str> {
    if let Some(kind) = value.get("kind").and_then(|v| v.as_str()) {
        return Some(kind);
    }
    let record = value.get("record").and_then(|v| v.as_str())?;
    match record {
        "outcome" => Some("outcome"),
        "intent" => Some("intent"),
        _ => None,
    }
}

fn note_path_hint(state: &mut CleanLogScanState, path: &str) {
    state.saw_path = true;
    if !super::is_sandbox_path_hint(path) {
        state.all_sandbox = false;
    }
}

fn inspect_clean_log_value(state: &mut CleanLogScanState, value: &serde_json::Value) {
    let kind = json_record_kind(value);
    let target_id = value
        .get("target_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let path = value
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match kind {
        Some("outcome") => {
            state.saw_outcome = true;
            state.outcomes.insert((target_id, path));
        }
        Some("intent") => state.intents.push((target_id, path)),
        _ => {}
    }

    if let Some(p) = value.get("path").and_then(|v| v.as_str()) {
        note_path_hint(state, p);
    }
    if let Some(target) = value.get("target_path").and_then(|v| v.as_str()) {
        note_path_hint(state, target);
    }
}

fn inspect_clean_log_line(state: &mut CleanLogScanState, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return;
    };
    inspect_clean_log_value(state, &value);
}

fn accumulate_clean_log_state(reader: BufReader<fs::File>) -> CleanLogScanState {
    let mut state = CleanLogScanState {
        all_sandbox: true,
        ..CleanLogScanState::default()
    };
    for line in reader.lines().map_while(Result::ok) {
        inspect_clean_log_line(&mut state, &line);
    }
    state
}

fn is_clean_log_file_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            lower.starts_with("clean-") && lower.ends_with(".jsonl")
        })
        .unwrap_or(false)
}

fn inspect_clean_log(path: &Path) -> Option<CleanLogScanState> {
    let file = fs::File::open(path).ok()?;
    Some(accumulate_clean_log_state(BufReader::new(file)))
}

fn should_include_log_file(path: &Path, size: u64, filter_sandbox_phantoms: bool) -> bool {
    if size == 0 {
        return false;
    }
    if is_clean_log_file_name(path) && filter_sandbox_phantoms {
        // An intent-without-outcome log is reconciled and surfaced rather than
        // hidden: a post-delete journal failure may have left it outcome-less
        // even though the deletion succeeded. Sandbox-only test logs remain
        // filtered from the real user's list.
        if let Some(state) = inspect_clean_log(path) {
            if state.sandbox_only() && state.intent_without_outcome() == 0 {
                return false;
            }
        }
    }
    true
}

/// Append a reconciliation note for intent-without-outcome records to the log view.
fn annotate_unreconciled_clean_log(path: &Path, content: String) -> String {
    let Some(state) = inspect_clean_log(path) else {
        return content;
    };
    let unreconciled = state.intent_without_outcome();
    if unreconciled == 0 {
        return content;
    }
    let mut out = content;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "{{\"record\":\"reconciliation\",\"status\":\"unknown\",\"reason\":\"verify_on_disk\",\"intent_without_outcome\":{unreconciled}}}\n"
    ));
    out
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
    out.sort_by(|a, b| {
        let ta = a.modified_unix_secs.as_ref().map(|v| v.0).unwrap_or(0);
        let tb = b.modified_unix_secs.as_ref().map(|v| v.0).unwrap_or(0);
        tb.cmp(&ta).then_with(|| a.id.cmp(&b.id))
    });
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
            let content = if is_clean_log_file_name(&p) {
                annotate_unreconciled_clean_log(&p, content)
            } else {
                content
            };
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

#[cfg(test)]
mod tests {
    use super::{inspect_clean_log, list_logs_in_dir_impl, read_log_in_dir};

    #[test]
    fn real_path_with_sweeper_test_substring_is_not_sandbox() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("clean-test.jsonl");
        let content = concat!(
            "{\"kind\":\"intent\",\"path\":\"C:\\\\Users\\\\Public\\\\Documents\\\\sweeper-test-results\\\\report.json\"}\n",
            "{\"kind\":\"outcome\",\"status\":\"deleted\",\"path\":\"C:\\\\Users\\\\Public\\\\Documents\\\\sweeper-test-results\\\\report.json\"}\n"
        );
        std::fs::write(&path, content).expect("write log");
        let state = inspect_clean_log(&path).expect("inspect log");
        assert!(state.saw_outcome);
        assert!(!state.sandbox_only());
        assert_eq!(state.intent_without_outcome(), 0);
    }

    #[test]
    fn intent_without_outcome_log_is_listed_and_reconciled_in_view() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_id = "clean-1700000000-1-1";
        let path = dir.path().join(format!("{log_id}.jsonl"));
        // A journal that lost its outcome write after a deletion: intent only.
        let content =
            "{\"record\":\"intent\",\"target_id\":\"t1\",\"path\":\"C:\\\\Temp\\\\a.tmp\"}\n";
        std::fs::write(&path, content).expect("write log");

        // Even with the real-user sandbox-phantom filter enabled, the
        // reconciliation path must surface the log instead of hiding it.
        let logs = list_logs_in_dir_impl(Some(dir.path()), true).expect("list logs");
        assert!(logs.iter().any(|l| l.id == log_id), "logs={logs:?}");

        let read = read_log_in_dir(Some(dir.path()), log_id).expect("read log");
        assert!(read.content.contains("\"record\":\"reconciliation\""));
        assert!(read.content.contains("\"status\":\"unknown\""));
        assert!(read.content.contains("\"reason\":\"verify_on_disk\""));
        assert!(read.content.contains("\"intent_without_outcome\":1"));
    }

    #[test]
    fn fully_outcome_log_has_no_reconciliation_note() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_id = "clean-1700000000-2-2";
        let path = dir.path().join(format!("{log_id}.jsonl"));
        let content = concat!(
            "{\"record\":\"intent\",\"target_id\":\"t1\",\"path\":\"C:\\\\Temp\\\\b.tmp\"}\n",
            "{\"record\":\"outcome\",\"target_id\":\"t1\",\"path\":\"C:\\\\Temp\\\\b.tmp\",\"status\":\"deleted\"}\n"
        );
        std::fs::write(&path, content).expect("write log");

        let read = read_log_in_dir(Some(dir.path()), log_id).expect("read log");
        assert!(!read.content.contains("\"record\":\"reconciliation\""));
    }
}
