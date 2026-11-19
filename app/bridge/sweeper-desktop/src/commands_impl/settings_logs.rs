use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use sweeper_contract::{
    ContractError, LogMetadataDto, LogReadResponse, SettingsDto, U64String,
    ValidateCustomPathResponse,
};

use sweeper_core::paths::{get_final_path_from_raw_handle, CanonicalPath};

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

/// Exact path equality except for the drive-letter case.
///
/// `is_direct_log_child` folds case, which is correct for ordinary Windows
/// paths but lets a case-variant sibling (e.g. `LOGS` vs `logs`) pass on a
/// filesystem with per-directory case sensitivity when the logs base itself is
/// a junction. Comparing the handle-derived parent to the validated base
/// byte-for-byte (after the drive prefix) closes that gap.
fn same_path_exact_except_drive(a: &CanonicalPath, b: &CanonicalPath) -> bool {
    if !a.drive_prefix().eq_ignore_ascii_case(b.drive_prefix()) {
        return false;
    }
    a.as_str()[2..] == b.as_str()[2..]
}

/// Open a candidate log file without following reparse points.
///
/// Uses the same `FILE_FLAG_OPEN_REPARSE_POINT` no-follow pattern as
/// `sweeper_winfs::open_for_inspection`, but requests read access so the bounded
/// content can be read from this very handle. A file symbolic link is therefore
/// opened as the link itself, never as its target.
fn open_log_file_no_follow(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

/// Reject any reparse point observed on the opened handle (symlink, junction,
/// cloud placeholder). This is the handle-level equivalent of the
/// `sweeper_winfs::inspect_reparse_point` checks used by the safety layers.
fn handle_is_reparse_point(file: &fs::File) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    file.metadata()
        .map(|meta| {
            (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 || meta.is_symlink()
        })
        .unwrap_or(true) // fail closed if the handle cannot be inspected
}

/// Resolve one candidate name. `None` means the candidate is skipped: an
/// overlong suffixed name (e.g. a 250-char ID plus ".jsonl") cannot be a single
/// path component, so the existing bare-file fallback must stay reachable.
fn resolve_log_candidate(
    logs_dir: &CanonicalPath,
    name: &str,
    is_bare: bool,
) -> Result<Option<CanonicalPath>, ContractError> {
    if name.len() > 255 {
        if is_bare {
            return Err(invalid_log_id());
        }
        return Ok(None);
    }
    let joined = join_log_file_name(logs_dir, name);
    let resolved = CanonicalPath::parse_lexical(&joined).map_err(|_| invalid_log_id())?;
    if !is_direct_log_child(logs_dir, &resolved) {
        return Err(invalid_log_id());
    }
    Ok(Some(resolved))
}

/// Parse the final path reported by `GetFinalPathNameByHandleW` into a
/// `CanonicalPath`. The API returns a `\\?\`-prefixed path (VOLUME_NAME_DOS),
/// which the lexical parser rejects; strip that prefix first. UNC and other
/// non-drive forms fail to parse and are therefore rejected (fail closed).
fn canonical_path_from_handle_final(final_path: &str) -> Option<CanonicalPath> {
    let stripped = final_path.strip_prefix(r"\\?\").unwrap_or(final_path);
    CanonicalPath::parse_lexical(stripped).ok()
}

/// Reject a configured logs path whose ANCESTORS include a reparse point
/// (junction, symlink, or mount point).
///
/// `FILE_FLAG_OPEN_REPARSE_POINT` suppresses reparse processing only for the
/// FINAL path component; ancestor junctions are still traversed when a handle is
/// opened, and `GetFinalPathNameByHandle` then reports the redirected target.
/// That would silently move the containment root away from the directory the
/// configured path actually names. 8.3 short names are NOT reparse points, so
/// genuine short-name segments remain allowed by this check. The logs directory
/// itself (the final component) is deliberately not examined here; case-variant
/// junction bases are handled by the exact-parent check in `validate_log_handle`.
///
/// Returns `true` when an ancestor is a reparse point OR cannot be inspected
/// (fail closed).
fn logs_path_has_reparse_ancestor(dir: &Path) -> bool {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    let mut current = dir;
    while let Some(parent) = current.parent() {
        // `Path::parent()` on a drive root returns None, so every visited
        // prefix is a real directory to inspect.
        let Ok(file) = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent)
        else {
            return true; // fail closed if an ancestor cannot be opened
        };
        let Ok(meta) = file.metadata() else {
            return true; // fail closed if an ancestor cannot be inspected
        };
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 || meta.is_symlink() {
            return true;
        }
        current = parent;
    }
    false
}

/// Resolve the configured logs directory to its final (long-name) path via a
/// no-follow handle, so containment compares like-for-like with the file-handle
/// final path produced by `validate_log_handle`.
///
/// On NTFS the configured directory string can contain an 8.3 short-name segment
/// (e.g. `RUNNER~1` under `%TEMP%` on CI runners), while
/// `GetFinalPathNameByHandle` returns fully resolved long names. Comparing a
/// lexical short-name base against a resolved long-name file path would reject
/// valid direct children. Resolving the directory through a
/// `FILE_FLAG_OPEN_REPARSE_POINT` handle (so Link Points in the directory path
/// are never followed) keeps both sides in the same namespace without relaxing
/// any reparse-point/junction containment check. `None` means the handle could
/// not be opened or the final path could not be parsed; the caller then falls
/// back to the lexical form and still enforces containment (fail closed).
fn canonicalize_logs_dir(dir: &Path) -> Option<CanonicalPath> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
        .ok()?;
    let final_path = get_final_path_from_raw_handle(handle.as_raw_handle()).ok()?;
    canonical_path_from_handle_final(&final_path)
}

/// Validate the already-open READ handle without reopening its path.
///
/// Containment is resolved from this very handle. `GetFinalPathNameByHandle`
/// follows ancestor junctions that were traversed when the handle was opened, so
/// an ancestor junction swapped after the open cannot make an out-of-directory
/// file pass containment: the final path still reflects the handle's real target.
/// The previous approach reopened the path for validation and therefore allowed
/// that swap to pass containment while content was read from the stale handle.
fn validate_log_handle(logs_dir: &CanonicalPath, file: &fs::File) -> Result<(), ContractError> {
    use std::os::windows::io::AsRawHandle;

    if handle_is_reparse_point(file) {
        return Err(ContractError::InvalidInput {
            reason: "log file is not a regular file".into(),
        });
    }
    let final_path =
        get_final_path_from_raw_handle(file.as_raw_handle()).map_err(|_| invalid_log_id())?;
    let resolved = canonical_path_from_handle_final(&final_path).ok_or_else(invalid_log_id)?;
    if !is_direct_log_child(logs_dir, &resolved) {
        return Err(invalid_log_id());
    }
    // `is_direct_log_child` folds case; a case-variant sibling under a junction
    // base could pass that check on a case-sensitive directory. Require the
    // handle-derived parent to equal the validated base exactly (drive prefix
    // case aside). Both are long-name, handle-derived paths, so genuine 8.3
    // short names already match here.
    let parent = resolved.parent().ok_or_else(invalid_log_id)?;
    if !same_path_exact_except_drive(&parent, logs_dir) {
        return Err(invalid_log_id());
    }
    Ok(())
}

/// Open and validate one resolved candidate. `None` means the entry does not
/// exist, so the next candidate can be tried.
fn open_validated_log_candidate(
    logs_dir: &CanonicalPath,
    path: &Path,
) -> Result<Option<fs::File>, ContractError> {
    // Open without following reparse points. A file symbolic link placed at this
    // path (including one swapped in after the lexical check) is opened as the
    // link itself and rejected by the handle checks in `validate_log_handle`.
    let file = match open_log_file_no_follow(path) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    validate_log_handle(logs_dir, &file)?;
    Ok(Some(file))
}

fn log_file_too_large() -> ContractError {
    ContractError::InvalidInput {
        reason: "log file too large".into(),
    }
}

/// Read a log through the already-opened handle with a hard byte cap.
///
/// The declared size is checked first, but the read also goes one byte beyond the
/// cap and the raw byte count is checked again BEFORE decoding. A file that grows
/// during the read therefore fails instead of returning a truncated JSON record,
/// and a multibyte character split at the cap boundary is reported as "too large"
/// rather than as a UTF-8 (NotFound) error.
fn read_bounded_log_content<R: Read>(
    reader: R,
    declared_size: u64,
) -> Result<String, ContractError> {
    if declared_size > MAX_LOG_READ_BYTES {
        return Err(log_file_too_large());
    }
    let mut raw = Vec::new();
    reader
        .take(MAX_LOG_READ_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|_| ContractError::NotFound {
            reason: "log not found".into(),
        })?;
    if raw.len() as u64 > MAX_LOG_READ_BYTES {
        return Err(log_file_too_large());
    }
    String::from_utf8(raw).map_err(|_| ContractError::NotFound {
        reason: "log not found".into(),
    })
}

/// `read_log` in a specific directory (testable).
///
/// Defense in depth (audit F38 / CodeRabbit 4618617742 / 4618853361): the ID is
/// restricted to a file name, the candidate path is parsed with the canonical path
/// parser, and the parsed path must lie exactly one segment below the logs directory.
/// The file is opened without following reparse points, its final path is resolved
/// from that same read handle (never by reopening the path, which would allow an
/// ancestor-junction TOCTOU swap), and content is read through the same handle.
/// Reads are size-bounded and files that grow past the cap are rejected instead of
/// returning a partial record.
pub fn read_log_in_dir(dir: Option<&Path>, log_id: &str) -> Result<LogReadResponse, ContractError> {
    validate_log_id(log_id)?;
    let Some(dir) = dir else {
        return Err(ContractError::NotFound {
            reason: "log not found".into(),
        });
    };

    // A reparse-point ancestor would redirect the containment root away from
    // the directory this path names (GetFinalPathNameByHandle follows ancestor
    // junctions). Fail closed rather than falling back to a lexical base.
    if logs_path_has_reparse_ancestor(dir) {
        return Err(invalid_log_id());
    }

    // Resolve the base to long names so the handle final path check below
    // compares like-for-like; a short-name base (8.3 alias in `%TEMP%`) would
    // otherwise reject valid direct children on the CI Windows runner.
    let logs_dir = canonicalize_logs_dir(dir)
        .or_else(|| CanonicalPath::parse_lexical(&dir.to_string_lossy()).ok())
        .ok_or_else(invalid_log_id)?;

    // Try the same extensions as `list_logs`, plus the bare ID.
    let candidates = [
        format!("{log_id}.jsonl"),
        format!("{log_id}.log"),
        log_id.to_string(),
    ];
    let bare_index = candidates.len() - 1;
    for (index, name) in candidates.into_iter().enumerate() {
        let Some(resolved) = resolve_log_candidate(&logs_dir, &name, index == bare_index)? else {
            continue;
        };
        let Some(file) = open_validated_log_candidate(&logs_dir, Path::new(resolved.as_str()))?
        else {
            continue;
        };
        let size = file.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        let content = read_bounded_log_content(file, size)?;
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
    use super::{
        clean_log_phantom_reasons, read_bounded_log_content, read_log_in_dir, MAX_LOG_READ_BYTES,
    };
    use std::io::Read;
    use sweeper_contract::ContractError;
    use sweeper_testkit::{create_file_symlink, Sandbox};

    /// True when a filesystem error means the sandbox lacks the privilege to
    /// perform the setup step, in which case the regression test skips itself.
    fn is_insufficient_privilege_error(error: &impl std::fmt::Display) -> bool {
        let message = error.to_string().to_lowercase();
        ["privilege", "permission", "denied"]
            .iter()
            .any(|needle| message.contains(needle))
    }

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
    fn read_log_reaches_bare_file_when_suffixed_candidate_is_overlong() {
        let outer = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox::new_in(outer.path());
        let logs = sandbox.create_dir("logs").expect("logs dir");

        // 250 ASCII chars + ".jsonl" = 256 bytes, over the 255-byte component
        // limit. The suffixed candidates must be skipped so the existing bare
        // file is still reachable.
        let bare_id = "a".repeat(250);
        std::fs::write(logs.join(&bare_id), b"bare-content").expect("write bare log");

        let resp = read_log_in_dir(Some(&logs), &bare_id).expect("long bare ID must resolve");
        assert_eq!(resp.id, bare_id);
        assert_eq!(resp.content, "bare-content");
    }

    #[test]
    fn read_log_rejects_file_symlink_outside_logs_dir() {
        let outer = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox::new_in(outer.path());
        let logs = sandbox.create_dir("logs").expect("logs dir");
        let secret = sandbox
            .write_file("outside-secret.jsonl", b"LEAK")
            .expect("secret file outside logs dir");
        let alias = sandbox.guarded_path("logs/alias.jsonl");

        match create_file_symlink(&alias, &secret) {
            Ok(()) => {}
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("privilege") || msg.contains("permission") {
                    eprintln!("skipping symlink regression test (insufficient privilege): {e}");
                    return;
                }
                panic!("create file symlink failed: {e}");
            }
        }

        let err = read_log_in_dir(Some(&logs), "alias").unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "file symlink to an outside file must be rejected, got {err:?}"
        );
    }

    /// Regression for CodeRabbit 4618853361 (ancestor-junction TOCTOU swap).
    ///
    /// The old code opened the read handle through an ancestor junction, then
    /// re-opened the path to check containment. If the junction was swapped in
    /// between, the second open saw a benign direct child and passed while the
    /// original handle still read a secret outside the logs directory. The fix
    /// resolves the final path of the ORIGINAL handle, so the swap cannot change
    /// what containment observes. A positive control (the benign replacement)
    /// proves the containment comparison itself still works against the real
    /// resolved base.
    #[test]
    #[cfg(windows)]
    fn read_log_rejects_original_handle_after_ancestor_junction_swap() {
        use std::io::Read;
        use sweeper_testkit::create_junction;

        let outer = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox::new_in(outer.path());
        let outside = sandbox.create_dir("outside").expect("outside dir");
        sandbox
            .write_file("outside/victim.jsonl", b"LEAK")
            .expect("secret outside logs dir");
        let logs = sandbox.guarded_path("logs");

        // `logs` is a junction pointing OUTSIDE when the read handle is opened.
        if let Err(e) = create_junction(&logs, &outside) {
            if is_insufficient_privilege_error(&e) {
                eprintln!("skipping junction swap regression test (insufficient privilege): {e}");
                return;
            }
            panic!("create junction failed: {e}");
        }

        // This is the "first handle" from the old implementation: its path is
        // `logs/victim.jsonl`, but it reads the secret through the junction.
        let candidate = logs.join("victim.jsonl");
        let file = super::open_log_file_no_follow(&candidate).expect("open through junction");
        {
            let mut reader = file.try_clone().expect("clone read handle");
            let mut content = String::new();
            reader
                .read_to_string(&mut content)
                .expect("read through junction");
            assert_eq!(
                content, "LEAK",
                "read handle should reach the outside secret"
            );
        }

        // Swap the junction aside and put a REAL logs directory in its place,
        // holding a benign direct child. `rename` moves only the junction entry
        // (it does not traverse into its target). The old path-based
        // re-validation would now see this file and pass containment while the
        // original handle still reads "LEAK".
        let swapped_away = sandbox.guarded_path("logs-junction-swapped-away");
        std::fs::rename(&logs, &swapped_away).expect("move junction aside");
        std::fs::create_dir_all(&logs).expect("create real logs dir");
        std::fs::write(&candidate, b"benign").expect("write benign direct child");

        // Resolve the real (post-swap) base the way `read_log_in_dir` does, so
        // the comparison uses a handle-derived, long-name path rather than the
        // lexical spelling saved before the swap.
        let real_base = super::canonicalize_logs_dir(&logs).expect("resolve real logs base");

        // POSITIVE CONTROL: a handle on the benign direct child must PASS
        // containment against that base. This proves the exact-parent comparison
        // works and is not rejecting everything.
        let benign = super::open_log_file_no_follow(&candidate).expect("open benign child");
        super::validate_log_handle(&real_base, &benign)
            .expect("benign direct child must pass containment against the resolved base");

        // The original handle still points at the outside secret, so it must
        // FAIL against the very same base.
        let err = super::validate_log_handle(&real_base, &file)
            .expect_err("original out-of-directory handle must be rejected after swap");
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "original handle to an outside file must be rejected, got {err:?}"
        );
    }

    #[test]
    fn read_log_rejects_content_that_grows_past_read_cap() {
        struct GrownReader {
            remaining: usize,
        }
        impl Read for GrownReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = buf.len().min(self.remaining);
                buf[..n].fill(b'x');
                self.remaining -= n;
                Ok(n)
            }
        }

        // The declared size looks small, but the reader yields cap+1 bytes, as
        // if the file grew between the metadata check and the bounded read.
        let err = read_bounded_log_content(
            GrownReader {
                remaining: (MAX_LOG_READ_BYTES + 1) as usize,
            },
            0,
        )
        .unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "grown log must fail instead of returning a truncated record, got {err:?}"
        );
    }

    /// A case-variant sibling of the logs base must not pass containment. The
    /// folded `is_self_or_ancestor_of` check accepts both spellings; the exact
    /// parent comparison rejects the `LOGS` variant. This is purely lexical, so
    /// it needs no case-sensitive-filesystem privileges.
    #[test]
    fn exact_parent_check_rejects_case_variant_sibling() {
        use sweeper_core::paths::CanonicalPath;

        let base = CanonicalPath::parse_lexical(r"C:\Temp\logs").expect("base");
        let good_child = CanonicalPath::parse_lexical(r"C:\Temp\logs\app.jsonl").expect("good");
        let bad_child = CanonicalPath::parse_lexical(r"C:\Temp\LOGS\app.jsonl").expect("bad");

        // The folded check alone cannot tell them apart.
        assert!(base.is_self_or_ancestor_of(&good_child));
        assert!(base.is_self_or_ancestor_of(&bad_child));

        // The exact comparison accepts only the literal base spelling.
        assert!(super::same_path_exact_except_drive(
            &good_child.parent().expect("good parent"),
            &base
        ));
        assert!(!super::same_path_exact_except_drive(
            &bad_child.parent().expect("bad parent"),
            &base
        ));
    }

    /// A file one byte over the cap whose extra byte is the first byte of a
    /// multibyte UTF-8 character must be reported as "too large", not as a
    /// UTF-8 decode (NotFound) error. The byte cap is enforced before decoding.
    #[test]
    fn read_log_rejects_multibyte_char_split_at_read_cap() {
        let mut bytes = vec![b'a'; MAX_LOG_READ_BYTES as usize];
        bytes.push(0xC3); // leading byte of a 2-byte sequence; partner absent
        let err = read_bounded_log_content(
            std::io::Cursor::new(bytes),
            MAX_LOG_READ_BYTES, // declared size looks legal
        )
        .unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "over-cap bytes must be rejected before UTF-8 decoding, got {err:?}"
        );
    }

    /// Valid UTF-8 that ends exactly at the cap still succeeds.
    #[test]
    fn read_log_accepts_valid_utf8_exactly_at_read_cap() {
        let mut bytes = vec![b'a'; (MAX_LOG_READ_BYTES - 2) as usize];
        bytes.extend_from_slice("ü".as_bytes()); // 2 bytes -> exactly MAX
        assert_eq!(bytes.len(), MAX_LOG_READ_BYTES as usize);
        let content = read_bounded_log_content(std::io::Cursor::new(bytes), MAX_LOG_READ_BYTES)
            .expect("valid UTF-8 exactly at cap must succeed");
        assert_eq!(content.len(), MAX_LOG_READ_BYTES as usize);
        assert!(content.ends_with('ü'));
    }

    /// A `~` segment in the logs path must not be treated as a reparse point:
    /// genuine 8.3 short names are ordinary directories, not junctions. This
    /// models the CI runner's `RUNNER~1` temp segment deterministically using a
    /// REAL directory whose literal name contains `~` (no junction involved).
    #[test]
    #[cfg(windows)]
    fn read_log_allows_tilde_segment_without_junction() {
        let outer = tempfile::tempdir().expect("tempdir");
        let tilde_root = outer.path().join("ROOTA~1");
        std::fs::create_dir_all(&tilde_root).expect("tilde-named directory");

        let logs = tilde_root.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("write log");

        let resp = read_log_in_dir(Some(&logs), "good")
            .expect("a literal ~ segment must be allowed (short names are not reparse points)");
        assert_eq!(resp.id, "good");
        assert_eq!(resp.content, "ok");
    }

    /// A junction ANCESTOR must be rejected: `FILE_FLAG_OPEN_REPARSE_POINT` only
    /// suppresses reparse processing for the final component, so an ancestor
    /// junction would otherwise redirect the containment root via
    /// `GetFinalPathNameByHandle`.
    #[test]
    #[cfg(windows)]
    fn read_log_rejects_junction_ancestor_of_logs_dir() {
        use sweeper_testkit::create_junction;

        let outer = tempfile::tempdir().expect("tempdir");
        let real_root = outer.path().join("real-root");
        std::fs::create_dir_all(&real_root).expect("real root");
        let alias_root = outer.path().join("ROOTA~1");
        if let Err(e) = create_junction(&alias_root, &real_root) {
            if is_insufficient_privilege_error(&e) {
                eprintln!(
                    "skipping ancestor-junction regression test (insufficient privilege): {e}"
                );
                return;
            }
            panic!("create junction failed: {e}");
        }

        let logs = alias_root.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("write log");

        let err = read_log_in_dir(Some(&logs), "good").unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "a reparse-point ancestor must redirect-containment rejected, got {err:?}"
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
