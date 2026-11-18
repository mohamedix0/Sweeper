use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use sweeper_contract::{
    ContractError, LogMetadataDto, LogReadResponse, SettingsDto, U64String,
    ValidateCustomPathResponse,
};

use sweeper_core::paths::CanonicalPath;

/// Upper bound on bytes returned by a single `read_log` IPC call (8 MiB).
/// Log files are small JSONL streams; bounding the read prevents a tampered or
/// runaway file from exhausting application memory.
const MAX_LOG_READ_BYTES: u64 = 8 * 1024 * 1024;

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
    if json_record_kind(value) == Some("outcome") {
        state.saw_outcome = true;
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

fn clean_log_phantom_reasons(path: &Path) -> (bool, bool) {
    let Ok(file) = fs::File::open(path) else {
        return (true, false);
    };
    let state = accumulate_clean_log_state(BufReader::new(file));
    let no_outcome = !state.saw_outcome;
    let sandbox_only = state.saw_path && state.all_sandbox;
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

fn invalid_log_id() -> ContractError {
    ContractError::InvalidInput {
        reason: "invalid log id".into(),
    }
}

/// `log_id` must be a bare file name: no drive qualifier (`:`), no separators,
/// no traversal, and no empty value. Dots within a name (e.g. `clean.2026`) and
/// Unicode names remain valid.
///
/// Each rule is a flat guard, so the predicate has no compound `||` conditional.
fn is_valid_log_id(log_id: &str) -> bool {
    if log_id.is_empty() {
        return false;
    }
    if log_id.contains("..") {
        return false;
    }
    if log_id.contains('/') {
        return false;
    }
    if log_id.contains('\\') {
        return false;
    }
    if log_id.contains(':') {
        return false;
    }
    true
}

fn validate_log_id(log_id: &str) -> Result<(), ContractError> {
    if is_valid_log_id(log_id) {
        Ok(())
    } else {
        Err(invalid_log_id())
    }
}

/// Join a candidate file name to the already-parsed logs directory.
fn join_log_file_name(logs_dir: &CanonicalPath, name: &str) -> String {
    if logs_dir.as_str().ends_with('\\') {
        format!("{}{name}", logs_dir.as_str())
    } else {
        format!("{}\\{name}", logs_dir.as_str())
    }
}

/// A resolved candidate is accepted only when it is a direct child of the logs
/// directory on the same drive. Each rule is a flat guard with no `||`.
fn is_direct_log_child(logs_dir: &CanonicalPath, resolved: &CanonicalPath) -> bool {
    if resolved.segment_count() != logs_dir.segment_count() + 1 {
        return false;
    }
    if !logs_dir.is_self_or_ancestor_of(resolved) {
        return false;
    }
    resolved
        .drive_prefix()
        .eq_ignore_ascii_case(logs_dir.drive_prefix())
}

/// `read_log` in a specific directory (testable).
///
/// Defense in depth (audit F38): the ID is restricted to a file name, the candidate
/// path is parsed with the canonical path parser, and the parsed path must lie
/// exactly one segment below the logs directory. Reads are size-bounded.
pub fn read_log_in_dir(dir: Option<&Path>, log_id: &str) -> Result<LogReadResponse, ContractError> {
    validate_log_id(log_id)?;
    let Some(dir) = dir else {
        return Err(ContractError::NotFound {
            reason: "log not found".into(),
        });
    };

    let logs_dir =
        CanonicalPath::parse_lexical(&dir.to_string_lossy()).map_err(|_| invalid_log_id())?;

    // Try the same extensions as `list_logs`, plus the bare ID.
    let candidates = [
        format!("{log_id}.jsonl"),
        format!("{log_id}.log"),
        log_id.to_string(),
    ];
    for name in candidates {
        let joined = join_log_file_name(&logs_dir, &name);
        let resolved = CanonicalPath::parse_lexical(&joined).map_err(|_| invalid_log_id())?;
        if !is_direct_log_child(&logs_dir, &resolved) {
            return Err(invalid_log_id());
        }

        let path = Path::new(resolved.as_str());
        if !path.is_file() {
            continue;
        }
        let file = fs::File::open(path).map_err(|_| ContractError::NotFound {
            reason: "log not found".into(),
        })?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        if size > MAX_LOG_READ_BYTES {
            return Err(ContractError::InvalidInput {
                reason: "log file too large".into(),
            });
        }
        let mut content = String::new();
        file.take(MAX_LOG_READ_BYTES)
            .read_to_string(&mut content)
            .map_err(|_| ContractError::NotFound {
                reason: "log not found".into(),
            })?;
        return Ok(LogReadResponse {
            id: log_id.to_string(),
            content,
        });
    }
    Err(ContractError::NotFound {
        reason: "log not found".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::{clean_log_phantom_reasons, read_log_in_dir, MAX_LOG_READ_BYTES};
    use sweeper_contract::ContractError;

    #[test]
    fn read_log_accepts_valid_dotted_and_unicode_file_name_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).expect("create logs dir");
        std::fs::write(logs.join("clean.2026.ünicode.jsonl"), b"line-one\n").expect("write log");

        let resp = read_log_in_dir(Some(&logs), "clean.2026.ünicode").expect("read log");
        assert_eq!(resp.id, "clean.2026.ünicode");
        assert_eq!(resp.content, "line-one\n");

        // The bare-ID candidate is supported too.
        std::fs::write(logs.join("plain-id"), b"bare").expect("write bare log");
        assert_eq!(
            read_log_in_dir(Some(&logs), "plain-id")
                .expect("read bare log")
                .content,
            "bare"
        );
    }

    #[test]
    fn read_log_rejects_drive_qualified_and_non_file_name_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).expect("create logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("write log");

        let bad_ids = [
            format!("{}:foo", "C"), // drive-qualified ID must not escape the logs dir
            "..".to_string(),
            "../outside".to_string(),
            "sub\\file".to_string(),
            "sub/file".to_string(),
            String::new(),
        ];
        for id in bad_ids {
            let err = read_log_in_dir(Some(&logs), &id).unwrap_err();
            assert!(
                matches!(err, ContractError::InvalidInput { .. }),
                "id={id:?} should be rejected as invalid input, got: {err:?}"
            );
        }
        // A valid ID still resolves after the hardened checks.
        assert_eq!(
            read_log_in_dir(Some(&logs), "good")
                .expect("valid log id")
                .content,
            "ok"
        );
    }

    #[test]
    fn read_log_caps_bytes_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).expect("create logs dir");
        let oversized = vec![b'x'; (MAX_LOG_READ_BYTES + 1) as usize];
        std::fs::write(logs.join("huge.jsonl"), &oversized).expect("write oversized log");

        let err = read_log_in_dir(Some(&logs), "huge").unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "oversized log must be rejected, got: {err:?}"
        );
    }

    #[test]
    fn real_path_with_sweeper_test_substring_is_not_sandbox() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("clean-test.jsonl");
        let content = concat!(
            "{\"kind\":\"intent\",\"path\":\"C:\\\\Users\\\\Public\\\\Documents\\\\sweeper-test-results\\\\report.json\"}\n",
            "{\"kind\":\"outcome\",\"status\":\"deleted\",\"path\":\"C:\\\\Users\\\\Public\\\\Documents\\\\sweeper-test-results\\\\report.json\"}\n"
        );
        std::fs::write(&path, content).expect("write log");
        let (no_outcome, sandbox_only) = clean_log_phantom_reasons(&path);
        assert!(!no_outcome);
        assert!(!sandbox_only);
    }
}
