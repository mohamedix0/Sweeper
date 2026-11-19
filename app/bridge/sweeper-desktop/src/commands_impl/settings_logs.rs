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

#[cfg(test)]
use super::log_ancestors::open_pinned_ancestors;
use super::log_ancestors::{
    check_logs_path_ancestors, pin_ancestors_with_retry, AncestorCheck, PinnedAncestors,
};
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

pub(super) fn invalid_log_id() -> ContractError {
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
/// any reparse-point/junction containment check. The caller must hold the
/// handles from `open_pinned_ancestors` across this call so the ancestors that
/// `GetFinalPathNameByHandle` traverses cannot be swapped for junctions first.
/// `None` means the handle could not be opened or the final path could not be
/// parsed; the caller then falls back to the lexical form and still enforces
/// containment (fail closed).
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

/// Authoritative post-resolution check for attribute-level ancestor swaps.
///
/// The restrictive pin share mode is only defence in depth: Windows does NOT
/// apply share modes to `FILE_WRITE_ATTRIBUTES` access, and that access is all
/// `FSCTL_SET_REPARSE_POINT` needs. A process can therefore turn a pinned
/// ancestor into a junction DURING the pin window. When that happens the base
/// resolved by `canonicalize_logs_dir` travels through the junction and its
/// ancestor chain no longer matches the chain captured by the pinned handles.
/// Compare every component and fail closed. Both sides are handle-derived
/// long names, so genuine 8.3 short names compare equal.
fn resolved_base_matches_pins(pinned: &PinnedAncestors, base: &CanonicalPath) -> bool {
    let expected = pinned.ancestor_final_paths();
    let mut actual: Vec<CanonicalPath> = Vec::with_capacity(expected.len());
    let mut current = base.clone();
    while let Some(parent) = current.parent() {
        if parent.is_drive_root() {
            break;
        }
        actual.push(parent.clone());
        current = parent;
    }
    if actual.len() != expected.len() {
        return false;
    }
    actual.reverse();
    actual.iter().zip(expected).all(|(segment, recorded)| {
        canonical_path_from_handle_final(recorded).is_some_and(|recorded| segment == &recorded)
    })
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
/// Ancestors are pinned root-to-leaf with WRITE/DELETE sharing denied before base
/// resolution. The file is opened without following reparse points, its final path
/// is resolved from that same read handle (never by reopening the path, which would
/// allow an ancestor-junction TOCTOU swap), and content is read through the same
/// handle. Reads are size-bounded and files that grow past the cap are rejected
/// instead of returning a partial record.
///
/// Review 5969166919 (self-review of the whole chain): the pin's share mode is
/// NOT the guarantee — `FILE_WRITE_ATTRIBUTES` access bypasses share modes and
/// suffices for `FSCTL_SET_REPARSE_POINT`. The AUTHORITATIVE check is
/// `resolved_base_matches_pins`, which compares the freshly resolved base's
/// ancestor chain to the chain captured at pin time. All other swap windows are
/// covered by existing handle-derived checks: a junction on the base itself or
/// on any ancestor AFTER resolution makes the file handle's final-path parent
/// differ from the base (`validate_log_handle`), and a reparse-point file is
/// rejected by `handle_is_reparse_point`. Both sides are long-name, handle-derived
/// paths, so genuine 8.3 short names stay valid.
pub fn read_log_in_dir(dir: Option<&Path>, log_id: &str) -> Result<LogReadResponse, ContractError> {
    read_log_in_dir_after_check(dir, log_id, || ())
}

/// Testable core of `read_log_in_dir`.
///
/// `after_ancestor_check` runs after the preliminary ancestor check but before
/// the authoritative pinned re-check and base resolution. Production passes a
/// no-op; the junction-swap regressions install a junction in this window to
/// prove that the pinned re-check rejects it instead of resolving a redirected
/// containment base.
fn read_log_in_dir_after_check<F: FnOnce()>(
    dir: Option<&Path>,
    log_id: &str,
    after_ancestor_check: F,
) -> Result<LogReadResponse, ContractError> {
    read_log_in_dir_with_hooks(dir, log_id, after_ancestor_check, || ())
}

/// Testable core with an additional hook that runs AFTER the pin is established
/// but before base resolution. Production passes a no-op; the attribute-only
/// junction-swap regression uses it to convert a pinned ancestor in place and
/// prove that the post-resolution chain comparison rejects the redirected base.
fn read_log_in_dir_with_hooks<F: FnOnce(), G: FnOnce()>(
    dir: Option<&Path>,
    log_id: &str,
    after_ancestor_check: F,
    after_pin: G,
) -> Result<LogReadResponse, ContractError> {
    validate_log_id(log_id)?;
    let Some(dir) = dir else {
        return Err(ContractError::NotFound {
            reason: "log not found".into(),
        });
    };

    // A reparse-point ancestor would redirect the containment root away from
    // the directory this path names (GetFinalPathNameByHandle follows ancestor
    // junctions). Fail closed rather than falling back to a lexical base. A
    // sharing violation here is retryable contention, not an invalid id; the
    // bounded pin helper re-runs this check on every attempt.
    if matches!(check_logs_path_ancestors(dir), AncestorCheck::Unsafe) {
        return Err(invalid_log_id());
    }

    after_ancestor_check();

    // Authoritative root-to-leaf pin with a bounded, validation-restarting retry
    // on sharing violation. Holding these handles until the end of this function
    // prevents the check-then-use swap that the preliminary check alone could not
    // stop. A contention failure is reported distinctly, never as an invalid id.
    let _pinned = pin_ancestors_with_retry(dir)?;

    after_pin();

    // Resolve the base to long names so the handle final path check below
    // compares like-for-like; a short-name base (8.3 alias in `%TEMP%`) would
    // otherwise reject valid direct children on the CI Windows runner.
    let resolved = canonicalize_logs_dir(dir);
    // AUTHORITATIVE check (review 5969166919): a pinned ancestor can be
    // converted to a junction via FILE_WRITE_ATTRIBUTES access, which share
    // modes cannot deny. If that happened, the freshly resolved base now sits
    // under a different ancestor chain; compare it to the chain captured at pin
    // time and fail closed on ANY component change. This does not rely on share
    // modes at all.
    if let Some(resolved_dir) = &resolved {
        if !resolved_base_matches_pins(&_pinned, resolved_dir) {
            return Err(invalid_log_id());
        }
    }
    let logs_dir = resolved
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

    /// Regression for review 5967742829: an ancestor junction installed AFTER
    /// the preliminary ancestor check must be caught by the authoritative
    /// pinned re-check at base resolution. Without the fix, `canonicalize_logs_dir`
    /// followed that junction and `GetFinalPathNameByHandle` reported the
    /// redirected target as the base, so `victim.jsonl` passed containment.
    ///
    /// Fully deterministic: the swap is a real rename + junction creation in the
    /// test's own sandbox, with no registry or 8.3-short-name dependence.
    #[test]
    #[cfg(windows)]
    fn read_log_rejects_ancestor_junction_installed_after_ancestor_check() {
        use sweeper_testkit::create_junction;

        let outer = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox::new_in(outer.path());

        // `real/logs` is the configured logs directory. `outside/logs` holds a
        // victim file that a successful swap would expose as a direct child.
        let real = sandbox.create_dir("real").expect("real dir");
        let logs = real.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("benign log");

        let outside = sandbox.create_dir("outside").expect("outside dir");
        let outside_logs = outside.join("logs");
        std::fs::create_dir_all(&outside_logs).expect("outside logs dir");
        std::fs::write(outside_logs.join("victim.jsonl"), b"LEAK").expect("victim");

        // Ordinary success control: the benign direct child reads before swap.
        assert_eq!(
            read_log_in_dir(Some(&logs), "good")
                .expect("ordinary logs dir must still read")
                .content,
            "ok"
        );

        // Probe junction creation BEFORE renaming the ancestor. If this sandbox
        // lacks the required privilege, skip cleanly instead of leaving `real`
        // renamed away (which would panic at `create_junction(...).expect(...)`).
        let probe = sandbox.guarded_path("junction-privilege-probe");
        if let Err(e) = create_junction(&probe, &outside) {
            if is_insufficient_privilege_error(&e) {
                eprintln!(
                    "skipping ancestor-junction swap regression test (insufficient privilege): {e}"
                );
                return;
            }
            panic!("junction privilege probe failed: {e}");
        }
        // The throwaway probe junction lives in `outer` and is cleaned up with
        // the TempDir; no explicit removal is needed (Gate A forbids fs::remove_dir).

        // Install the junction at the ancestor after the preliminary check but
        // before base resolution. `rename` moves only the directory entry; it
        // does not traverse into the target.
        let swapped_away = sandbox.guarded_path("real-swapped-away");
        let install_junction = || {
            std::fs::rename(&real, &swapped_away).expect("rename real ancestor aside");
            create_junction(&real, &outside).expect("install ancestor junction");
        };

        let err = super::read_log_in_dir_after_check(Some(&logs), "victim", install_junction)
            .unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "ancestor junction installed after the check must be rejected, got {err:?}"
        );
    }

    /// A genuine 8.3 short-name alias is an ordinary directory, not a reparse
    /// point. A real directory with a `~` in its name is a deterministic
    /// stand-in (no registry dependence) and must stay accepted by the pinned
    /// ancestor check, and reads below it must keep working.
    #[test]
    #[cfg(windows)]
    fn pinned_ancestors_accept_tilde_named_directory() {
        let outer = tempfile::tempdir().expect("tempdir");
        let tilde_root = outer.path().join("ROOTA~1");
        std::fs::create_dir_all(&tilde_root).expect("tilde-named directory");
        let logs = tilde_root.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("write log");

        assert!(
            super::open_pinned_ancestors(&logs).is_ok(),
            "a ~-named directory is not a reparse point and must be pinned/accepted"
        );
        assert_eq!(
            read_log_in_dir(Some(&logs), "good")
                .expect("short-name alias must keep working")
                .content,
            "ok"
        );
    }

    /// Ordinary (non-junction) ancestors must pin successfully so the safety
    /// check does not reject normal logs directories.
    #[test]
    #[cfg(windows)]
    fn pinned_ancestors_accept_ordinary_logs_dir() {
        let outer = tempfile::tempdir().expect("tempdir");
        let logs = outer.path().join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        assert!(
            super::open_pinned_ancestors(&logs).is_ok(),
            "ordinary ancestors must be accepted by the pinned re-check"
        );
    }

    /// A pinned ancestor must deny WRITE sharing. Setting a junction needs WRITE
    /// access to the directory, so granting WRITE sharing would let another
    /// process convert a pinned ancestor in place during the pin window (the
    /// bug fixed here). READ sharing stays enabled so reads/traversal still work.
    #[test]
    #[cfg(windows)]
    fn pinned_ancestors_deny_write_sharing() {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const ERROR_SHARING_VIOLATION: i32 = 32;

        let outer = tempfile::tempdir().expect("tempdir");
        let real = outer.path().join("real");
        std::fs::create_dir_all(&real).expect("real dir");
        let logs = real.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");

        // Control: WRITE access to the ancestor is grantable when nothing pins it.
        std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&real)
            .expect("control: unpinned ancestor must be openable for write");

        let pinned = super::open_pinned_ancestors(&logs).expect("pin ancestors");

        // While pinned, the WRITE open is refused with a sharing violation: an
        // attacker cannot get the write handle needed to set a reparse point.
        let write = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&real);
        assert_eq!(
            write.err().and_then(|e| e.raw_os_error()),
            Some(ERROR_SHARING_VIOLATION),
            "a pinned ancestor must deny WRITE sharing (junction install needs write access)"
        );

        // READ sharing is preserved; reads and ancestor traversal still work.
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&real)
            .expect("read sharing must remain enabled on pinned ancestors");

        drop(pinned);
    }

    /// Another process holding a conflicting handle on an ordinary ancestor
    /// makes Windows reject the pin even though nothing moved. That must surface
    /// as a distinct, retryable contention error, NOT as "invalid log id".
    #[test]
    #[cfg(windows)]
    fn read_log_reports_sharing_violation_as_retryable_contention() {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

        let outer = tempfile::tempdir().expect("tempdir");
        let real = outer.path().join("real");
        std::fs::create_dir_all(&real).expect("real dir");
        let logs = real.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("write log");

        // Hold WRITE access on the `real` ancestor with READ-only sharing. The
        // preliminary check (read, share-all) still succeeds, but the restrictive
        // pin (read, share-read-only) is rejected with ERROR_SHARING_VIOLATION.
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&real)
            .expect("hold conflicting write handle on ancestor");

        let err = read_log_in_dir(Some(&logs), "good").unwrap_err();
        assert!(
            matches!(err, ContractError::Internal { .. }),
            "sharing violation must be distinct retryable contention, not invalid log id: {err:?}"
        );
        if let ContractError::Internal { reason } = &err {
            assert!(
                reason.to_lowercase().contains("retr"),
                "contention reason should advise retry: {reason}"
            );
        }

        drop(holder);
        assert_eq!(
            read_log_in_dir(Some(&logs), "good")
                .expect("read must succeed once contention clears")
                .content,
            "ok"
        );
    }

    /// A sharing violation in the PRELIMINARY ancestor check (before the pin)
    /// must also surface as retryable contention, not "invalid log id". This is
    /// the same misclassification shape as the pin case, one step earlier.
    #[test]
    #[cfg(windows)]
    fn read_log_reports_precheck_sharing_violation_as_contention() {
        use std::os::windows::fs::OpenOptionsExt;

        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

        let outer = tempfile::tempdir().expect("tempdir");
        let real = outer.path().join("real");
        std::fs::create_dir_all(&real).expect("real dir");
        let logs = real.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("write log");

        // This handle's share mode denies READ, so the preliminary ancestor
        // check's read open fails with a sharing violation (before the pin).
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&real)
            .expect("hold read-denying handle on ancestor");

        let err = read_log_in_dir(Some(&logs), "good").unwrap_err();
        assert!(
            matches!(err, ContractError::Internal { .. }),
            "preliminary sharing violation must be contention, not invalid log id: {err:?}"
        );

        drop(holder);
        assert_eq!(
            read_log_in_dir(Some(&logs), "good")
                .expect("read must succeed once contention clears")
                .content,
            "ok"
        );
    }

    /// A junction swapped into a HIGHER ancestor after the preliminary check
    /// must be caught by the root-to-leaf pin re-check. Without root-to-leaf
    /// acquisition, a deeper absolute-path open traverses that higher ancestor
    /// without checking it, so the redirected base could pass containment.
    #[test]
    #[cfg(windows)]
    fn read_log_rejects_higher_ancestor_junction_installed_after_check() {
        use sweeper_testkit::create_junction;

        let outer = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox::new_in(outer.path());

        // `root/real/logs` is the configured logs directory. The victim lives
        // under `outside/root/real/logs`, exposed only if the higher `root`
        // ancestor is replaced by a junction to `outside/root`.
        let root = sandbox.create_dir("root").expect("higher ancestor");
        let real = root.join("real");
        std::fs::create_dir_all(&real).expect("parent dir");
        let logs = real.join("logs");
        std::fs::create_dir_all(&logs).expect("logs dir");
        std::fs::write(logs.join("good.jsonl"), b"ok").expect("benign log");

        let outside = sandbox.create_dir("outside").expect("outside dir");
        let outside_root = outside.join("root");
        let outside_logs = outside_root.join("real").join("logs");
        std::fs::create_dir_all(&outside_logs).expect("outside logs dir");
        std::fs::write(outside_logs.join("victim.jsonl"), b"LEAK").expect("victim");

        assert_eq!(
            read_log_in_dir(Some(&logs), "good")
                .expect("ordinary logs dir must still read")
                .content,
            "ok"
        );

        // Probe junction privilege before any rename (see sibling tests).
        let probe = sandbox.guarded_path("junction-privilege-probe-higher");
        if let Err(e) = create_junction(&probe, &outside) {
            if is_insufficient_privilege_error(&e) {
                eprintln!("skipping higher-ancestor swap test (insufficient privilege): {e}");
                return;
            }
            panic!("junction privilege probe failed: {e}");
        }
        // The throwaway probe junction is cleaned up with the TempDir.

        let swapped_away = sandbox.guarded_path("root-swapped-away");
        let install_higher_junction = || {
            std::fs::rename(&root, &swapped_away).expect("rename higher ancestor aside");
            create_junction(&root, &outside_root).expect("install higher-ancestor junction");
        };

        let err =
            super::read_log_in_dir_after_check(Some(&logs), "victim", install_higher_junction)
                .unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "higher-ancestor junction installed after the check must be rejected, got {err:?}"
        );
    }

    /// Review 5969166919: the pin's `FILE_SHARE_READ` is only defence in depth —
    /// Windows does not apply share modes to `FILE_WRITE_ATTRIBUTES` access, and
    /// that is all `FSCTL_SET_REPARSE_POINT` needs. While the pin is held, open
    /// the deepest pinned ancestor with attribute-only access, install a junction
    /// on it, and assert the post-resolution ancestor-chain check rejects the
    /// redirected base. The deepest ancestor is left empty at pin time so the
    /// reparse point can be installed in place (no rename/delete needed).
    #[test]
    #[cfg(windows)]
    fn read_log_rejects_attribute_only_junction_swap_while_pinned() {
        use std::cell::Cell;
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use sweeper_testkit::{create_junction, set_junction_on_open_handle};

        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

        let outer = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox::new_in(outer.path());

        // `real/logs` is the configured logs dir; `real` is the deepest pinned
        // ancestor and is intentionally empty at pin time. `outside/logs/victim`
        // is exposed only if `real` becomes a junction to `outside`.
        let real = sandbox.create_dir("real").expect("real dir");
        let logs = real.join("logs");
        let outside = sandbox.create_dir("outside").expect("outside dir");
        let outside_logs = outside.join("logs");
        std::fs::create_dir_all(&outside_logs).expect("outside logs dir");
        std::fs::write(outside_logs.join("victim.jsonl"), b"LEAK").expect("victim");

        // Probe junction privilege before any mutation, skipping cleanly on a
        // sandbox that cannot create junctions (same helper as sibling tests).
        let probe = sandbox.guarded_path("junction-privilege-probe-attr");
        if let Err(e) = create_junction(&probe, &outside) {
            if is_insufficient_privilege_error(&e) {
                eprintln!("skipping attribute-only swap test (insufficient privilege): {e}");
                return;
            }
            panic!("junction privilege probe failed: {e}");
        }

        let swapped = Cell::new(false);
        let install_attribute_junction = || {
            // This open is expected to SUCCEED despite the pin holding
            // `real` with FILE_SHARE_READ: share modes do not gate attribute
            // access. If this host denies it, skip instead of failing the test.
            let handle = match std::fs::OpenOptions::new()
                .access_mode(FILE_WRITE_ATTRIBUTES | SYNCHRONIZE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(&real)
            {
                Ok(handle) => handle,
                Err(e) if is_insufficient_privilege_error(&e) => {
                    eprintln!("skipping attribute-only swap test (open denied): {e}");
                    return;
                }
                Err(e) => panic!("attribute-only open of pinned ancestor failed: {e}"),
            };
            match set_junction_on_open_handle(handle.as_raw_handle(), &outside) {
                Ok(()) => swapped.set(true),
                Err(e) if is_insufficient_privilege_error(&e) => {
                    eprintln!("skipping attribute-only swap test (FSCTL denied): {e}");
                }
                Err(e) => panic!("install junction via attribute-only handle failed: {e}"),
            }
        };

        let err = super::read_log_in_dir_with_hooks(
            Some(&logs),
            "victim",
            || (),
            install_attribute_junction,
        )
        .unwrap_err();
        if !swapped.get() {
            // The bypass could not be demonstrated on this host: skip the
            // assertion rather than reporting a false failure.
            return;
        }
        assert!(
            matches!(err, ContractError::InvalidInput { .. }),
            "attribute-only ancestor junction installed after the pin must be rejected, got {err:?}"
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
