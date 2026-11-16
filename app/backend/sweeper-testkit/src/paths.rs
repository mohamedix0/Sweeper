use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Selects the default base directory for test sandboxes.
///
/// Resolution order:
/// 1. `SWEEPER_SANDBOX_BASE` (explicit, documented override), if set and safe
/// 2. Canonicalized process temp directory, if safe
/// 3. Cargo target sandbox locations (`CARGO_TARGET_TMPDIR`/`CARGO_TARGET_DIR`/workspace target), if safe
pub fn default_sandbox_base() -> PathBuf {
    default_sandbox_base_checked().unwrap_or_else(|e| panic!("{e}"))
}

/// Non-panicking sandbox base resolution with actionable diagnostics.
pub fn default_sandbox_base_checked() -> io::Result<PathBuf> {
    if let Some(base) = resolve_configured_sandbox_base()? {
        return Ok(base);
    }
    if let Some(base) = resolve_temp_sandbox_base() {
        return Ok(base);
    }
    if let Some(base) = resolve_cargo_target_sandbox_base() {
        return Ok(base);
    }
    missing_sandbox_base_error()
}

fn resolve_configured_sandbox_base() -> io::Result<Option<PathBuf>> {
    let Some(v) = std::env::var_os("SWEEPER_SANDBOX_BASE") else {
        return Ok(None);
    };
    let raw = PathBuf::from(&v);
    let base = select_configured_base(&raw).ok_or_else(|| unsafe_configured_base_error(&raw))?;
    let _ = fs::create_dir_all(&base);
    Ok(Some(base))
}

fn select_configured_base(raw: &Path) -> Option<PathBuf> {
    let normalized = normalize_sandbox_base_path(raw.to_path_buf());
    if is_safe_sandbox_base(&normalized) {
        return Some(normalized);
    }
    // If normalization resolved/canonicalized to an unsafe location (e.g. junction outside
    // Users pointing into Users), reject instead of falling back to the raw path.
    if resolved_to_unsafe_sandbox_location(&normalized) {
        return None;
    }
    if is_safe_sandbox_base(raw) {
        return Some(raw.to_path_buf());
    }
    None
}

fn resolved_to_unsafe_sandbox_location(normalized: &Path) -> bool {
    is_under_protected_container_root_for_sandbox(normalized)
        || has_parent_dir_component(normalized)
        || has_short_name_component(normalized)
}

fn unsafe_configured_base_error(raw: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "SWEEPER_SANDBOX_BASE is set but unsafe for sandboxing: '{}'. \
Set SWEEPER_SANDBOX_BASE to a safe directory outside protected container roots (not under Users/Windows/ProgramData, drive root, or browser User Data), \
or set CARGO_TARGET_DIR to a safe workspace target and retry.",
            raw.display()
        ),
    )
}

fn resolve_temp_sandbox_base() -> Option<PathBuf> {
    let canonical_temp = canonicalize_sandbox_base(std::env::temp_dir());
    if is_safe_sandbox_base(&canonical_temp) {
        return Some(canonical_temp);
    }
    None
}

fn resolve_cargo_target_sandbox_base() -> Option<PathBuf> {
    let base = cargo_target_sandbox_base()?;
    let canonical_base = canonicalize_sandbox_base(base.clone());
    if is_safe_sandbox_base(&canonical_base) {
        let _ = fs::create_dir_all(&canonical_base);
        return Some(canonical_base);
    }
    if is_safe_sandbox_base(&base) {
        let _ = fs::create_dir_all(&base);
        return Some(base);
    }
    None
}

fn missing_sandbox_base_error() -> io::Result<PathBuf> {
    let canonical_temp = canonicalize_sandbox_base(std::env::temp_dir());
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "no safe sandbox base found (temp '{}' is unsafe for sandboxing; no safe cargo-target fallback was available). \
Set SWEEPER_SANDBOX_BASE to a safe directory outside protected container roots, or set CARGO_TARGET_DIR to a safe workspace target and retry.",
            canonical_temp.display()
        ),
    ))
}

fn cargo_target_sandbox_base() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CARGO_TARGET_TMPDIR") {
        return Some(normalize_sandbox_base_path(
            PathBuf::from(v).join("sweeper-test-sandboxes"),
        ));
    }
    if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
        return Some(normalize_sandbox_base_path(
            PathBuf::from(v).join("sweeper-test-sandboxes"),
        ));
    }

    // Best-effort workspace-local fallback for testkit users outside cargo-target tmp contexts.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fallback = manifest.join("..\\..\\..\\target\\sweeper-test-sandboxes");
    Some(normalize_sandbox_base_path(fallback))
}

pub(crate) fn strip_extended_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    p
}

pub(crate) fn normalize_sandbox_base_path(base: PathBuf) -> PathBuf {
    // Ensure directories exist so canonicalization can resolve .. and remove short-name aliases.
    let _ = fs::create_dir_all(&base);
    if let Ok(c) = fs::canonicalize(&base) {
        return strip_extended_prefix(c);
    }
    // Fallback: lexical normalization (collapse ..) without touching FS beyond best effort.
    let normalized: PathBuf = base.components().collect();
    let _ = fs::create_dir_all(&normalized);
    if let Ok(c) = fs::canonicalize(&normalized) {
        return strip_extended_prefix(c);
    }
    strip_extended_prefix(normalized)
}

fn canonicalize_sandbox_base(base: PathBuf) -> PathBuf {
    normalize_sandbox_base_path(base)
}

/// Returns true if `path` is safe to use as a Sweeper test sandbox base.
///
/// A safe sandbox base must not contain short-name (`~`) or `..` components and must not be
/// located at or under protected container roots that would cause analyzer/policy to classify
/// sandbox contents as protected/unselectable (drive roots and `C:\Windows`, `C:\Users`,
/// `C:\ProgramData`, plus browser `User Data` container roots). Both normal and extended-length
/// (`\\?\`) canonical Windows path forms are handled, case-insensitively.
pub fn is_safe_sandbox_base(path: &Path) -> bool {
    !has_short_name_component(path)
        && !has_parent_dir_component(path)
        && !is_under_protected_container_root_for_sandbox(path)
}

fn is_under_protected_container_root_for_sandbox(path: &Path) -> bool {
    let normalized = strip_extended_prefix(path.to_path_buf());
    let parts = normal_path_segments(&normalized);
    if is_drive_root_path(&normalized, &parts) {
        return true;
    }
    if is_protected_top_level_container(&parts) {
        return true;
    }
    parts.iter().any(|s| s.eq_ignore_ascii_case("User Data"))
}

fn normal_path_segments(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(os) => Some(os.to_string_lossy().to_string()),
            _ => None,
        })
        .collect()
}

fn is_drive_root_path(path: &Path, parts: &[String]) -> bool {
    parts.is_empty()
        && path
            .components()
            .any(|c| matches!(c, std::path::Component::RootDir))
}

fn is_protected_top_level_container(parts: &[String]) -> bool {
    let Some(first) = parts.first() else {
        return false;
    };
    matches!(
        first.to_ascii_uppercase().as_str(),
        "WINDOWS" | "USERS" | "PROGRAMDATA"
    )
}

fn has_short_name_component(path: &Path) -> bool {
    path.components().any(|c| match c {
        std::path::Component::Normal(os) => os.to_string_lossy().contains('~'),
        _ => false,
    })
}

fn has_parent_dir_component(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

pub(crate) fn to_extended_path_for_removal(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if s.starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        if let Some(rest) = s.strip_prefix(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{}", rest));
        }
        PathBuf::from(format!(r"\\?\{}", s))
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        has_parent_dir_component, has_short_name_component, is_safe_sandbox_base,
        normalize_sandbox_base_path,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn short_name_component_detection_flags_tilde_segments() {
        assert!(has_short_name_component(Path::new(
            r"C:\Temp\SHORT~1\AppData\Local\Temp"
        )));
        assert!(!has_short_name_component(Path::new(
            r"C:\Temp\longname\AppData\Local\Temp"
        )));
    }

    #[test]
    fn safe_sandbox_base_requires_no_short_name_segments() {
        assert!(!is_safe_sandbox_base(Path::new(
            r"C:\Temp\SHORT~1\AppData\Local\Temp"
        )));
        assert!(is_safe_sandbox_base(Path::new(
            r"D:\work\target\sweeper-test-sandboxes"
        )));
    }

    #[test]
    fn normalized_sandbox_base_has_no_parent_dir_components() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let p = manifest.join("..\\..\\..\\target\\sweeper-test-sandboxes");
        let norm = normalize_sandbox_base_path(p);
        assert!(
            !has_parent_dir_component(&norm),
            "normalized path must not contain '..' components: {}",
            norm.display()
        );
        assert!(
            !has_short_name_component(&norm),
            "normalized path must not contain short-name '~' segments: {}",
            norm.display()
        );
    }
}
