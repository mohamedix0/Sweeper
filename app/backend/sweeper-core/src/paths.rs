//! Path parsing, normalization, and validated path types (Parse, Don't Validate).

use std::fmt;
use std::path::Path;

use sweeper_winfs::{self, ReparseTagClass, WinfsError};
use thiserror::Error;

/// Errors produced by lexical parsing or on-disk canonicalization.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Empty input.
    #[error("path is empty")]
    Empty,
    /// Contains NUL byte.
    #[error("path contains NUL byte")]
    NulByte,
    /// Root-only or missing required structure.
    #[error("path is a root or has insufficient structure")]
    RootOrInsufficient,
    /// Not an absolute drive path.
    #[error("path must be an absolute drive path (e.g. C:\\...)")]
    NotAbsoluteDrivePath,
    /// UNC path rejected.
    #[error("UNC paths are not allowed")]
    Unc,
    /// Device/extended/verbatim prefixes rejected.
    #[error("device or extended path prefixes are not allowed")]
    DevicePrefix,
    /// Drive-relative path (e.g. C:foo).
    #[error("drive-relative paths are not allowed")]
    DriveRelative,
    /// Traversal components (`.`/`..`) rejected.
    #[error("path traversal components are not allowed")]
    Traversal,
    /// Trailing space or dot in a segment.
    #[error("path segments must not end with space or dot")]
    TrailingSpaceOrDot,
    /// Alternate Data Stream syntax.
    #[error("alternate data streams are not allowed")]
    AlternateDataStream,
    /// Reserved DOS device name.
    #[error("reserved DOS device name")]
    ReservedDeviceName,
    /// Control, bidi, or otherwise disallowed Unicode.
    #[error("disallowed Unicode characters")]
    DisallowedUnicode,
    /// 8.3 short-name alias usage (input differs from canonical resolved name).
    #[error("8.3 short-name alias is not allowed")]
    ShortNameAlias,
    /// Path exceeds maximum supported length.
    #[error("path exceeds maximum length")]
    TooLong,
    /// Segment is empty.
    #[error("empty path segment")]
    EmptySegment,
    /// Filesystem/open/canonicalization error.
    #[error("filesystem resolution failed: {0}")]
    Fs(String),
    /// Path is a link point (symlink/junction) and must not be used.
    #[error("path is a link point")]
    LinkPoint,
    /// Path does not exist.
    #[error("path does not exist")]
    NotFound,
}

/// A lexically parsed and normalized absolute Windows drive path.
///
/// Private construction ensures callers only obtain this type through parsing.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CanonicalPath {
    /// Normalized string form (e.g. `C:\Foo\Bar`).
    normalized: String,
    /// Uppercased normalized form for case-insensitive comparisons.
    folded: String,
    /// Path segments after the drive root (e.g. ["Foo","Bar"]).
    segments: Vec<String>,
}

impl fmt::Debug for CanonicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CanonicalPath")
            .field("normalized", &self.normalized)
            .finish()
    }
}

impl CanonicalPath {
    /// Parse and normalize a path lexically (no filesystem access).
    ///
    /// Accepts absolute drive paths with `/` or `\` separators and normalizes to `\`.
    /// Rejects adversarial/ambiguous forms required by Gate C.
    pub fn parse_lexical(input: &str) -> Result<Self, PathError> {
        lexical_parse(input)
    }

    /// Resolve an on-disk path to a canonical validated path using handle APIs.
    ///
    /// Performs lexical validation, requires existence, opens without following reparse points,
    /// rejects link points, and resolves the final canonical path via `GetFinalPathNameByHandle`.
    /// Also rejects short (8.3) names when the resolved long path differs by name component shape.
    pub fn from_existing_path(input: &str) -> Result<ValidatedPath, PathError> {
        let lexical = Self::parse_lexical(input)?;
        // Ensure it exists and is not a link point using handle inspection.
        let handle = sweeper_winfs::open_for_inspection(Path::new(lexical.as_str()))
            .map_err(map_winfs_err)?;
        let reparse = sweeper_winfs::inspect_reparse_point(&handle).map_err(map_winfs_err)?;
        if matches!(reparse, ReparseTagClass::LinkPoint) {
            return Err(PathError::LinkPoint);
        }
        let final_path = sweeper_winfs::get_final_path(&handle).map_err(map_winfs_err)?;
        if final_path.starts_with(r"\\?\UNC\") {
            return Err(PathError::Unc);
        }
        let final_str = strip_win32_namespace_prefix(&final_path);
        if final_str.starts_with("\\\\") {
            return Err(PathError::Unc);
        }
        let resolved = Self::parse_lexical(final_str)?;
        // Reject 8.3 short-name usage: if input's final component appears short-named but
        // resolved differs in a way indicating aliasing, require the caller to use canonical name.
        reject_short_name_alias(input, resolved.as_str())?;
        Ok(ValidatedPath { resolved })
    }

    /// Normalized path string.
    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    /// Number of segments after the drive root.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Drive letter prefix in normalized form (e.g. `C:`).
    pub fn drive_prefix(&self) -> &str {
        &self.normalized[..2]
    }

    /// Case-insensitive, segment-aware comparison: is `self` the same as or ancestor of `other`?
    pub fn is_self_or_ancestor_of(&self, other: &CanonicalPath) -> bool {
        if self.folded == other.folded {
            return true;
        }
        if other.segments.len() <= self.segments.len() {
            return false;
        }
        if !other.folded.starts_with(&self.folded) {
            return false;
        }
        // Segment boundary: next char in other after self must be '\'
        other
            .normalized
            .as_bytes()
            .get(self.normalized.len())
            .copied()
            == Some(b'\\')
    }

    /// Case-insensitive segment-aware equality.
    pub fn is_same_path(&self, other: &CanonicalPath) -> bool {
        self.folded == other.folded
    }

    /// Returns true if the drive root (e.g. `C:\`).
    pub fn is_drive_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// Segments iterator.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().map(|s| s.as_str())
    }
}

impl fmt::Display for CanonicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normalized)
    }
}

/// A path validated against on-disk state (exists, canonicalized, not a link point).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedPath {
    resolved: CanonicalPath,
}

impl ValidatedPath {
    /// Canonical resolved path.
    pub fn canonical(&self) -> &CanonicalPath {
        &self.resolved
    }

    /// Normalized string form.
    pub fn as_str(&self) -> &str {
        self.resolved.as_str()
    }

    /// Consume into the canonical path.
    pub fn into_canonical(self) -> CanonicalPath {
        self.resolved
    }
}

fn lexical_parse(input: &str) -> Result<CanonicalPath, PathError> {
    if input.is_empty() {
        return Err(PathError::Empty);
    }
    if input.as_bytes().contains(&0) {
        return Err(PathError::NulByte);
    }
    if input.len() > 32_767 {
        // Guard against pathological inputs; allow legitimate extended-length NTFS paths.
        return Err(PathError::TooLong);
    }
    check_disallowed_unicode(input)?;

    // Reject extended/device/UNC/verbatim prefixes early (raw forms).
    if input.starts_with("\\\\?\\") || input.starts_with("\\\\.\\") || input.starts_with("\\??\\") {
        return Err(PathError::DevicePrefix);
    }
    if input.starts_with("\\\\") {
        // Could be UNC (\\server\share) or device-like \\?\ after normalization attempts;
        // raw \\ is UNC/verbatim root in general and not allowed here.
        return Err(PathError::Unc);
    }

    // Normalize separators for structural checks.
    let replaced = input.replace('/', "\\");
    // Detect ADS ':' outside drive prefix position.
    // Allowed colon only at index 1 for drive letter.
    for (i, ch) in replaced.char_indices() {
        if ch == ':' && i != 1 {
            return Err(PathError::AlternateDataStream);
        }
    }

    // Basic drive-absolute form: X:\...
    let bytes = replaced.as_bytes();
    if bytes.len() < 2 {
        return Err(PathError::NotAbsoluteDrivePath);
    }
    let drive = bytes[0] as char;
    if !drive.is_ascii_alphabetic() || bytes[1] != b':' {
        return Err(PathError::NotAbsoluteDrivePath);
    }
    if bytes.len() == 2 {
        // "C:" drive-relative/root-ambiguous without separator
        return Err(PathError::DriveRelative);
    }
    if bytes[2] != b'\\' {
        // "C:foo" drive-relative
        return Err(PathError::DriveRelative);
    }

    // Disallow any embedded verbatim/device marker remnants and GLOBALROOT.
    let upper_check = replaced.to_ascii_uppercase();
    if upper_check.contains("\\??\\") || upper_check.contains("GLOBALROOT") {
        return Err(PathError::DevicePrefix);
    }

    // Parse components, reject traversal and validate segments.
    let mut segments: Vec<String> = Vec::new();
    // Skip "C:\" prefix
    let rest = &replaced[3..];
    for raw_seg in rest.split('\\') {
        if raw_seg.is_empty() {
            // reject empty segments (e.g. C:\a\\b) and trailing separator creating empty tail handled below
            return Err(PathError::EmptySegment);
        }
        if raw_seg.len() > 255 {
            // Guard against pathological component lengths while allowing deep trees.
            return Err(PathError::TooLong);
        }
        if raw_seg == "." || raw_seg == ".." {
            return Err(PathError::Traversal);
        }
        validate_segment(raw_seg)?;
        segments.push(raw_seg.to_string());
    }

    // Reject root-only in some contexts? Caller may decide, but CanonicalPath represents parsed absolute path.
    // For drive root "C:\" we would have returned EmptySegment if split produced empty; handle explicitly.
    // (rest cannot be empty here except "C:\" case which would make split yield [""] -> EmptySegment)

    let normalized = if segments.is_empty() {
        format!("{}:\\", drive.to_ascii_uppercase())
    } else {
        format!("{}:\\{}", drive.to_ascii_uppercase(), segments.join("\\"))
    };
    // Length is already bounded by the input-length and per-segment checks above.
    debug_assert!(normalized.len() <= 32_767);
    let folded = normalized.to_ascii_uppercase();
    Ok(CanonicalPath {
        normalized,
        folded,
        segments,
    })
}

/// Parse a drive root like `C:\`.
pub fn parse_drive_root(input: &str) -> Result<CanonicalPath, PathError> {
    if input.is_empty() {
        return Err(PathError::Empty);
    }
    if input.as_bytes().contains(&0) {
        return Err(PathError::NulByte);
    }
    check_disallowed_unicode(input)?;
    let replaced = input.replace('/', "\\");
    let bytes = replaced.as_bytes();
    if bytes.len() == 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let normalized = format!("{}:\\", drive);
        let folded = normalized.clone();
        return Ok(CanonicalPath {
            normalized,
            folded,
            segments: Vec::new(),
        });
    }
    Err(PathError::RootOrInsufficient)
}

fn validate_segment(seg: &str) -> Result<(), PathError> {
    // Trailing spaces or dots
    if seg.ends_with(' ') || seg.ends_with('.') {
        return Err(PathError::TrailingSpaceOrDot);
    }
    // Control chars already checked globally, but be strict per segment.
    for ch in seg.chars() {
        if ch.is_control() {
            return Err(PathError::DisallowedUnicode);
        }
    }
    // Reserved DOS device names (base name before optional extension).
    let upper = seg.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or(&upper);
    if is_reserved_dos_name(base) {
        return Err(PathError::ReservedDeviceName);
    }
    Ok(())
}

fn is_reserved_dos_name(base_upper: &str) -> bool {
    matches!(
        base_upper,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) || is_reserved_dos_name_unicode_variant(base_upper)
}

fn is_reserved_dos_name_unicode_variant(base_upper: &str) -> bool {
    // Superscript digits/letters variants sometimes used to bypass checks (e.g. CON¹, LPT¹).
    // Normalize common superscripts/subscripts in base and re-check.
    let mut normalized = String::with_capacity(base_upper.len());
    for ch in base_upper.chars() {
        let mapped = match ch {
            '¹' => '1',
            '²' => '2',
            '³' => '3',
            '⁴' => '4',
            '⁵' => '5',
            '⁶' => '6',
            '⁷' => '7',
            '⁸' => '8',
            '⁹' => '9',
            _ => ch,
        };
        normalized.push(mapped);
    }
    matches!(
        normalized.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn check_disallowed_unicode(input: &str) -> Result<(), PathError> {
    for ch in input.chars() {
        if ch == '\0' {
            return Err(PathError::NulByte);
        }
        // Control characters (including DEL)
        if ch.is_control() || ch == '\u{007F}' {
            return Err(PathError::DisallowedUnicode);
        }
        // Bidi overrides/isolates/marks and related controls commonly abused
        if matches!(
            ch,
            '\u{200E}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{061C}'
                | '\u{FEFF}'
        ) {
            return Err(PathError::DisallowedUnicode);
        }
        // Fullwidth slash/backslash/colon/dot tricks that can normalize confusingly in some contexts:
        // reject to be conservative (spec says reject Unicode normalization/full-width/bidi/control characters)
        if matches!(ch, '／' | '＼' | '：' | '．') {
            return Err(PathError::DisallowedUnicode);
        }
    }
    Ok(())
}

fn strip_win32_namespace_prefix(path: &str) -> &str {
    // \\?\UNC\server\share -> \\server\share
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        // Return a view isn't possible with leading \\ added without allocation; caller checks for \\ prefix
        // by also handling \\?\UNC\ specially via original prefix detection in from_existing_path.
        // But to keep simple, detect in caller; here we return rest as-is and caller treats specially.
        return rest;
    }
    path.strip_prefix(r"\\?\")
        .or_else(|| path.strip_prefix(r"\\.\"))
        .unwrap_or(path)
}

fn map_winfs_err(e: WinfsError) -> PathError {
    match e {
        WinfsError::Win32 { code: 2 | 3, .. } => PathError::NotFound, // FILE_NOT_FOUND/PATH_NOT_FOUND
        other => PathError::Fs(other.to_string()),
    }
}

fn reject_short_name_alias(
    original_input: &str,
    resolved_canonical: &str,
) -> Result<(), PathError> {
    // Heuristic/strict check: if any segment in original input contains '~' (common 8.3 marker),
    // require that the resolved final component/path representation not differ in a way that indicates
    // a short-name alias was used. If mismatch, reject to force canonical long-name usage.
    let orig_norm = original_input.replace('/', "\\");
    if !orig_norm.contains('~') {
        return Ok(());
    }
    // Compare case-insensitively on normalized forms after stripping extended prefix possibility.
    let orig_trim = orig_norm.trim_end_matches('\\');
    let res_trim = resolved_canonical.trim_end_matches('\\');
    if orig_trim.eq_ignore_ascii_case(res_trim) {
        return Ok(());
    }
    // Also allow if only prefix differs (e.g. \\?\) already stripped by caller; but resolved_canonical has no prefix.
    // A differing name with '~' present implies short-name aliasing -> reject.
    Err(PathError::ShortNameAlias)
}

/// Segment-aware, case-insensitive check: does `candidate` start at/under `base`?
///
/// Prevents prefix bugs (`C:\Windows` must not match `C:\Windows.old`).
pub fn is_same_or_under(candidate: &CanonicalPath, base: &CanonicalPath) -> bool {
    base.is_self_or_ancestor_of(candidate)
}

/// Check if `ancestor` is a strict ancestor of `descendant` (segment-aware, case-insensitive).
pub fn is_strict_ancestor(ancestor: &CanonicalPath, descendant: &CanonicalPath) -> bool {
    is_same_or_under(descendant, ancestor) && !ancestor.is_same_path(descendant)
}

/// Utility: returns true if path string is > 260 chars (legacy MAX_PATH threshold), relevant to policy decisions.
pub fn exceeds_legacy_max_path_len(s: &str) -> bool {
    s.len() > 260
}

/// Convenience for tests/fuzzing: parse lexical and return normalized string.
pub fn normalize_lexical(input: &str) -> Result<String, PathError> {
    Ok(CanonicalPath::parse_lexical(input)?.normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(s: &str) -> CanonicalPath {
        CanonicalPath::parse_lexical(s).expect("valid lexical path")
    }

    #[test]
    fn segment_aware_prefix() {
        let base = canon(r"C:\Windows");
        let a = canon(r"C:\Windows.old");
        let b = canon(r"C:\WindowsApps");
        let c = canon(r"C:\Windows\Temp");
        assert!(!is_same_or_under(&a, &base));
        assert!(!is_same_or_under(&b, &base));
        assert!(is_same_or_under(&c, &base));
        assert!(is_same_or_under(&base, &base));
    }

    #[test]
    fn rejects_short_name_alias_marker_when_mismatched() {
        assert!(matches!(
            reject_short_name_alias(r"C:\PROGRA~1\Foo", r"C:\Program Files\Foo"),
            Err(PathError::ShortNameAlias)
        ));
        assert!(reject_short_name_alias(r"C:\Temp\LongName", r"C:\Temp\LongName").is_ok());
    }

    #[test]
    fn debug_fmt_includes_canonical_path() {
        let c = canon(r"C:\Sandbox\Dir");
        let dbg = format!("{c:?}");
        assert!(
            dbg.contains("CanonicalPath") && dbg.contains("normalized") && dbg.contains("Sandbox"),
            "Debug output must include canonical path info, got: {dbg}"
        );
    }

    #[test]
    fn drive_prefix_is_exact_drive() {
        assert_eq!(canon(r"C:\Sandbox\Dir").drive_prefix(), "C:");
        assert_eq!(canon(r"d:\Sandbox").drive_prefix(), "D:");
    }

    #[test]
    fn lexical_parse_length_boundaries() {
        // Overall input length boundary: > 32_767 rejected, 32_767 allowed.
        fn build_path_len(target_len: usize) -> String {
            let mut s = String::from(r"C:\");
            while s.len() < target_len {
                let rem = target_len - s.len();
                if s.ends_with('\\') {
                    let seg_len = std::cmp::min(rem, 250);
                    s.push_str(&"a".repeat(seg_len));
                } else if rem == 1 {
                    s.push('a');
                } else {
                    let seg_len = std::cmp::min(rem - 1, 250);
                    s.push('\\');
                    s.push_str(&"a".repeat(seg_len));
                }
            }
            assert_eq!(s.len(), target_len);
            s
        }

        let ok = build_path_len(32_767);
        assert!(
            CanonicalPath::parse_lexical(&ok).is_ok(),
            "input length 32767 should be allowed"
        );

        let too_long = build_path_len(32_768);
        assert!(matches!(
            CanonicalPath::parse_lexical(&too_long),
            Err(PathError::TooLong)
        ));
    }

    #[test]
    fn lexical_parse_rejects_device_and_unc_prefixes_independently() {
        assert!(matches!(
            CanonicalPath::parse_lexical(r"\\?\C:\Temp"),
            Err(PathError::DevicePrefix)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"\\.\C:\Temp"),
            Err(PathError::DevicePrefix)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"\??\C:\Temp"),
            Err(PathError::DevicePrefix)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"\\server\share\file"),
            Err(PathError::Unc)
        ));
    }

    #[test]
    fn lexical_parse_drive_form_boundaries() {
        // len < 2 -> NotAbsoluteDrivePath (e.g. "C")
        assert!(matches!(
            CanonicalPath::parse_lexical("C"),
            Err(PathError::NotAbsoluteDrivePath)
        ));
        // len == 2 -> DriveRelative ("C:")
        assert!(matches!(
            CanonicalPath::parse_lexical("C:"),
            Err(PathError::DriveRelative)
        ));
        // drive not alphabetic or missing colon
        assert!(matches!(
            CanonicalPath::parse_lexical(r"1:\Temp"),
            Err(PathError::NotAbsoluteDrivePath)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"CC\Temp"),
            Err(PathError::NotAbsoluteDrivePath)
        ));
        // "C:foo" drive-relative
        assert!(matches!(
            CanonicalPath::parse_lexical("C:foo"),
            Err(PathError::DriveRelative)
        ));
    }

    #[test]
    fn lexical_parse_rejects_embedded_device_markers_independently() {
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\Temp\??\x"),
            Err(PathError::DevicePrefix)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\Temp\GLOBALROOT\Device"),
            Err(PathError::DevicePrefix)
        ));
    }

    #[test]
    fn lexical_parse_segment_boundaries_and_traversal() {
        // segment length 255 ok, 256 TooLong
        let seg255 = "a".repeat(255);
        let ok = format!(r"C:\{seg255}");
        assert!(CanonicalPath::parse_lexical(&ok).is_ok());
        let seg256 = "a".repeat(256);
        let bad = format!(r"C:\{seg256}");
        assert!(matches!(
            CanonicalPath::parse_lexical(&bad),
            Err(PathError::TooLong)
        ));

        // traversal components rejected individually
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\."),
            Err(PathError::Traversal)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\.."),
            Err(PathError::Traversal)
        ));
        // empty segment
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\a\\b"),
            Err(PathError::EmptySegment)
        ));
    }

    #[test]
    fn lexical_parse_normalized_length_boundary() {
        // Long but valid segmented paths must be accepted (normalized-length guard must not
        // reject valid long paths within parser limits).
        let mut s = String::from(r"C:\");
        while s.len() < 8_000 {
            let rem = 8_000 - s.len();
            if s.ends_with('\\') {
                let seg_len = std::cmp::min(rem, 200);
                s.push_str(&"a".repeat(seg_len));
            } else if rem == 1 {
                s.push('a');
            } else {
                let seg_len = std::cmp::min(rem - 1, 200);
                s.push('\\');
                s.push_str(&"a".repeat(seg_len));
            }
        }
        assert_eq!(s.len(), 8_000);
        let parsed = CanonicalPath::parse_lexical(&s).expect("long path should parse");
        assert_eq!(parsed.as_str().len(), 8_000);
        assert_eq!(normalize_lexical(&s).unwrap().len(), 8_000);
    }

    #[test]
    fn reserved_dos_names_and_unicode_variants() {
        // Basic reserved names rejected
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\CON"),
            Err(PathError::ReservedDeviceName)
        ));
        assert!(matches!(
            CanonicalPath::parse_lexical(r"C:\LPT1.txt"),
            Err(PathError::ReservedDeviceName)
        ));

        // Unicode superscript variants map to reserved names (each arm matters).
        for name in [
            "COM\u{00B9}",
            "COM\u{00B2}",
            "COM\u{00B3}",
            "COM\u{2074}",
            "COM\u{2075}",
            "COM\u{2076}",
            "COM\u{2077}",
            "COM\u{2078}",
            "COM\u{2079}",
        ] {
            let p = format!(r"C:\{name}");
            assert!(
                matches!(
                    CanonicalPath::parse_lexical(&p),
                    Err(PathError::ReservedDeviceName)
                ),
                "expected reserved for {name}"
            );
        }
        for name in [
            "LPT\u{00B9}",
            "LPT\u{00B2}",
            "LPT\u{00B3}",
            "LPT\u{2074}",
            "LPT\u{2075}",
            "LPT\u{2076}",
            "LPT\u{2077}",
            "LPT\u{2078}",
            "LPT\u{2079}",
        ] {
            let p = format!(r"C:\{name}");
            assert!(
                matches!(
                    CanonicalPath::parse_lexical(&p),
                    Err(PathError::ReservedDeviceName)
                ),
                "expected reserved for {name}"
            );
        }
        // Superscript zero variant is not mapped to a reserved DOS device name; it must remain allowed.
        assert!(CanonicalPath::parse_lexical("C:\\COM\u{2070}").is_ok());
        assert!(CanonicalPath::parse_lexical("C:\\LPT\u{2070}").is_ok());
        // Non-reserved unicode variant-like names return false/are allowed.
        assert!(CanonicalPath::parse_lexical(r"C:\CONX").is_ok());
        assert!(CanonicalPath::parse_lexical("C:\\FOO\u{00B9}").is_ok());
    }

    #[test]
    fn is_strict_ancestor_semantics() {
        let parent = canon(r"C:\Temp");
        let child = canon(r"C:\Temp\Sub");
        let same = canon(r"C:\Temp");
        let sibling = canon(r"C:\Temp2");
        assert!(is_strict_ancestor(&parent, &child));
        assert!(!is_strict_ancestor(&same, &same));
        assert!(!is_strict_ancestor(&sibling, &parent));
        assert!(!is_strict_ancestor(&child, &parent));
    }

    #[test]
    fn exceeds_legacy_max_path_len_boundary() {
        let s260 = "a".repeat(260);
        let s261 = "a".repeat(261);
        assert!(!exceeds_legacy_max_path_len(&s260));
        assert!(exceeds_legacy_max_path_len(&s261));
        assert!(!exceeds_legacy_max_path_len("short"));
    }

    #[test]
    fn rejects_non_del_control_characters() {
        // C0 control chars (except NUL, which is caught as NulByte) must be rejected as disallowed Unicode.
        // This distinguishes `is_control() || ch == DEL` from `is_control() && ch == DEL`.
        for c in ['\u{0001}', '\u{001F}', '\u{007F}', '\u{0080}', '\u{009F}'] {
            let p = format!("C:\\Temp\\foo{c}bar");
            assert!(
                matches!(
                    CanonicalPath::parse_lexical(&p),
                    Err(PathError::DisallowedUnicode)
                ),
                "expected disallowed unicode for control char U+{:04X}",
                c as u32
            );
        }
    }

    #[test]
    fn normalize_lexical_returns_expected_string() {
        assert_eq!(
            normalize_lexical(r"c:/Sandbox/Sub").unwrap(),
            r"C:\Sandbox\Sub"
        );
        assert_eq!(
            normalize_lexical(r"C:\Sandbox\Sub").unwrap(),
            r"C:\Sandbox\Sub"
        );
        assert!(matches!(
            normalize_lexical(r"\\server\share"),
            Err(PathError::Unc)
        ));
    }
}
