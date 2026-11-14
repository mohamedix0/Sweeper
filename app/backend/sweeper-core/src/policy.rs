//! Policy checks: protected paths and custom path validation.

use std::path::Path;

use sweeper_winfs::{self, ReparseTagClass};

use crate::paths::{self, CanonicalPath, PathError};

/// Verdict from validation APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationVerdict {
    /// Whether the path is allowed.
    pub allowed: bool,
    /// Human-readable reason (safe to expose at contract layer with care).
    pub reason: String,
}

impl ValidationVerdict {
    fn allow() -> Self {
        Self {
            allowed: true,
            reason: "ok".to_string(),
        }
    }
    fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
        }
    }
}

/// Classification of a protected path match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedClass {
    /// Subtree-protected: path or descendants (and ancestors in selection contexts per rule) blocked.
    Subtree,
    /// Protected container root: exact identity blocked for Custom Path / Analyzer selection.
    ContainerRoot,
}

/// Check if a canonical path is a drive root (e.g. `C:\`).
pub fn is_drive_root(path: &CanonicalPath) -> bool {
    path.is_drive_root()
}

/// Returns true if `path` is under or equal to any fixed subtree-protected prefix.
fn matches_fixed_subtree_protected(path: &CanonicalPath) -> bool {
    for p in fixed_subtree_protected_prefixes() {
        if let Ok(base) = CanonicalPath::parse_lexical(p) {
            if base.is_self_or_ancestor_of(path) {
                return true;
            }
        } else if let Ok(base) = paths::parse_drive_root(p) {
            if base.is_self_or_ancestor_of(path) {
                return true;
            }
        }
    }
    // Pattern-based subtree protections
    if has_segment_named(path, ".git")
        || has_segment_named(path, ".ssh")
        || has_segment_named(path, ".gnupg")
        || has_segment_named(path, ".aws")
        || has_segment_named(path, ".kube")
        || has_segment_named(path, ".docker")
    {
        return true;
    }
    // Recovery anywhere? spec lists `Recovery` as subtree-protected (commonly C:\Recovery)
    if has_segment_named(path, "Recovery") {
        return true;
    }
    // System Volume Information
    if has_segment_named(path, "System Volume Information") {
        return true;
    }
    // NTUSER.DAT* files (by final segment prefix)
    if let Some(last) = path_last_segment(path) {
        let u = last.to_ascii_uppercase();
        if u == "NTUSER.DAT" || u.starts_with("NTUSER.DAT") {
            return true;
        }
        if matches!(u.as_str(), "PAGEFILE.SYS" | "HIBERFIL.SYS" | "SWAPFILE.SYS") {
            return true;
        }
    }
    // Common user data folders under any user profile: Desktop/Documents/Downloads/Pictures/Videos/Music
    // and OneDrive sync roots (best-effort lexical: segment named OneDrive or OneDrive*)
    if is_user_profile_data_subtree(path) {
        return true;
    }
    false
}

/// Returns true if selecting `path` as a Custom Path/Analyzer target would be blocked because
/// `path` is an ancestor of any subtree-protected location (rule: "no Target root or Custom Path
/// may be an ancestor of any Subtree-Protected Path").
pub fn is_ancestor_of_subtree_protected(path: &CanonicalPath) -> bool {
    // Check fixed protected paths
    for p in fixed_subtree_protected_prefixes() {
        if let Ok(prot) = CanonicalPath::parse_lexical(p).or_else(|_| paths::parse_drive_root(p)) {
            if path.is_self_or_ancestor_of(&prot) && !path.is_same_path(&prot) {
                return true;
            }
            // equality handled by subtree-protected match itself
            if path.is_self_or_ancestor_of(&prot) && path.is_same_path(&prot) {
                return true;
            }
        }
    }
    // Pattern ancestors: selecting C:\Users or C:\Users\<name> could be ancestor of .ssh/.git under them,
    // but those are handled by container root/user profile rules and generic ancestor containment checks
    // using known important locations; be conservative for common cases.
    // If path is drive root, it is ancestor of many protected paths.
    if path.is_drive_root() {
        return true;
    }
    // C:\Users ancestor of all profile secret dirs -> but C:\Users is container root (blocked exactly)
    // and C:\Users\<name> container root.
    // For generic directories, we cannot know if they contain a future .git; but spec states
    // "any .git directory" subtree-protected. Lexical ancestor-of-.git can't be proven without FS,
    // however Custom Path must not contain a Subtree-Protected descendant (checked on-disk at validation time).
    // For lexical/property purposes, treat obvious ancestors of fixed user paths as risky? keep minimal here.
    false
}

/// Fixed subtree-protected prefixes (canonical forms). Includes some broad ones; user-specific ones
/// are supplemented by pattern rules.
fn fixed_subtree_protected_prefixes() -> &'static [&'static str] {
    &[
        r"C:\Windows\System32",
        r"C:\Windows\WinSxS",
        r"C:\Windows\SysWOW64",
        r"C:\Windows\Boot",
        r"C:\Windows\Fonts",
        r"C:\Windows\servicing",
        r"C:\Program Files",
        r"C:\Program Files (x86)",
        r"C:\Recovery",
        r"C:\System Volume Information",
    ]
}

/// Returns true if path matches a protected container root (exact identity).
pub fn is_protected_container_root(path: &CanonicalPath) -> bool {
    if path.is_drive_root() {
        return true;
    }
    // Fixed container roots
    if eq_ci(path, r"C:\Windows") || eq_ci(path, r"C:\Users") || eq_ci(path, r"C:\ProgramData") {
        return true;
    }
    // User profile root: C:\Users\<name>
    if is_user_profile_root(path) {
        return true;
    }
    // Browser "User Data" and profile roots (Default, Profile *)
    if is_browser_container_root(path) {
        return true;
    }
    false
}

/// Returns true if `path` is blocked by protected-container-root selection rules.
///
/// - Drive roots: exact identity blocked only.
/// - `C:\Windows`, `C:\Users`, `C:\ProgramData`: exact identity and all descendants blocked from
///   Custom Path / Analyzer selection (non-Allowlist descendants).
/// - Any user profile root (`C:\Users\<name>`): exact identity and all descendants blocked.
/// - Browser `User Data` and profile roots (`Default`, `Profile *` under `User Data`): exact
///   identity and all descendants blocked.
pub fn is_under_protected_container_root(path: &CanonicalPath) -> bool {
    if path.is_drive_root() {
        return true;
    }
    for root in [r"C:\Windows", r"C:\Users", r"C:\ProgramData"] {
        if let Ok(r) = CanonicalPath::parse_lexical(root) {
            if r.is_self_or_ancestor_of(path) {
                return true;
            }
        }
    }
    // Browser container roots: "User Data" and descendants, and profile roots under User Data.
    let segs: Vec<&str> = path.segments().collect();
    if segs.iter().any(|s| s.eq_ignore_ascii_case("User Data")) {
        return true;
    }
    false
}

fn eq_ci(path: &CanonicalPath, lit: &str) -> bool {
    if let Ok(p) = CanonicalPath::parse_lexical(lit) {
        path.is_same_path(&p)
    } else if let Ok(p) = paths::parse_drive_root(lit) {
        path.is_same_path(&p)
    } else {
        false
    }
}

fn has_segment_named(path: &CanonicalPath, name: &str) -> bool {
    let needle = name.to_ascii_uppercase();
    path.segments().any(|s| s.to_ascii_uppercase() == needle)
}

fn path_last_segment(path: &CanonicalPath) -> Option<&str> {
    path.segments().last()
}

fn is_user_profile_root(path: &CanonicalPath) -> bool {
    // C:\Users\<name> => segments ["Users","<name>"]
    let segs: Vec<&str> = path.segments().collect();
    segs.len() == 2 && segs[0].eq_ignore_ascii_case("Users")
}

fn is_user_profile_data_subtree(path: &CanonicalPath) -> bool {
    let segs: Vec<&str> = path.segments().collect();
    if segs.len() < 3 {
        return false;
    }
    if !segs[0].eq_ignore_ascii_case("Users") {
        return false;
    }
    // segs[1] = username, segs[2...] may include known data folders or OneDrive roots
    let third = segs[2];
    if matches_ci_any(
        third,
        &[
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Videos",
            "Music",
        ],
    ) {
        return true;
    }
    // OneDrive sync roots (common names)
    let third_u = third.to_ascii_uppercase();
    if third_u == "ONEDRIVE"
        || third_u.starts_with("ONEDRIVE -")
        || third_u.starts_with("ONEDRIVE-")
    {
        return true;
    }
    false
}

fn matches_ci_any(s: &str, opts: &[&str]) -> bool {
    opts.iter().any(|o| s.eq_ignore_ascii_case(o))
}

fn is_browser_container_root(path: &CanonicalPath) -> bool {
    // Recognize common Chromium/Firefox container/profile roots by path shape.
    // Examples: ...\Google\Chrome\User Data, ...\User Data\Default, ...\User Data\Profile 1
    let segs: Vec<&str> = path.segments().collect();
    if segs.is_empty() {
        return false;
    }
    let last = segs[segs.len() - 1];
    let last_u = last.to_ascii_uppercase();
    if last_u == "USER DATA" {
        return true;
    }
    if last_u == "DEFAULT" {
        // Treat Default as browser profile root when under a User Data path.
        if segs.iter().any(|s| s.eq_ignore_ascii_case("User Data")) {
            return true;
        }
    }
    if last_u.starts_with("PROFILE ") && segs.iter().any(|s| s.eq_ignore_ascii_case("User Data")) {
        return true;
    }
    false
}

/// Determine if a path is subtree-protected (self or under subtree-protected).
pub fn is_subtree_protected(path: &CanonicalPath) -> bool {
    matches_fixed_subtree_protected(path)
}

/// Combined lexical check: protected for Custom Path / Analyzer selection?
/// - any subtree-protected path (including known pattern-based segments like `.git`/`.ssh`)
/// - any ancestor of fixed subtree-protected paths (cannot be selected as target root/custom path)
/// - any path at or under a protected container root (container roots have exact identity protected
///   and non-Allowlist descendants blocked from selection; Custom Paths are never allowlisted)
///
/// Note: this is lexical-only. It cannot prove absence of pattern-protected descendants inside an
/// otherwise-unprotected directory; full on-disk [`validate_custom_path`] performs that check.
pub fn is_protected_for_selection(path: &CanonicalPath) -> bool {
    if is_subtree_protected(path) {
        return true;
    }
    if is_under_protected_container_root(path) {
        return true;
    }
    if is_ancestor_of_subtree_protected(path) {
        return true;
    }
    false
}

/// Lexical-only custom path validation (no filesystem access).
///
/// Useful for fuzz/property tests and early rejection. **Lexical checks cannot detect
/// pattern-based subtree-protected descendants that may exist on disk** (e.g. a directory
/// containing a `.git`, `.ssh`, etc.). A path passing lexical checks must still pass full
/// on-disk validation via [`validate_custom_path`], which scans descendants without following
/// link points and rejects paths containing subtree-protected descendants.
pub fn validate_custom_path_lexical(input: &str) -> ValidationVerdict {
    let path = match CanonicalPath::parse_lexical(input) {
        Ok(p) => p,
        Err(e) => return ValidationVerdict::deny(format!("invalid path: {e}")),
    };
    // depth >= 3 segments
    if path.segment_count() < 3 {
        return ValidationVerdict::deny("custom path must have depth >= 3 segments");
    }
    if is_protected_for_selection(&path) {
        return ValidationVerdict::deny("path is protected or within/above a protected location");
    }
    ValidationVerdict::allow()
}

/// Full custom path validation per spec (§4):
/// exists on disk (no parent fallback), absolute without `..`, not a Link Point,
/// depth >= 3, neither Subtree-Protected nor Protected Container Root,
/// and contains no Subtree-Protected descendant.
pub fn validate_custom_path(input: &str) -> ValidationVerdict {
    let lexical = match ensure_safe_custom_path_syntax(input) {
        Ok(p) => p,
        Err(v) => return v,
    };
    if let Err(v) = ensure_not_protected_or_root(&lexical, false) {
        return v;
    }

    let resolved = match CanonicalPath::from_existing_path(input) {
        Ok(v) => v.into_canonical(),
        Err(PathError::NotFound) => {
            return ValidationVerdict::deny("path does not exist (no parent fallback)")
        }
        Err(PathError::LinkPoint) => return ValidationVerdict::deny("path is a link point"),
        Err(e) => return ValidationVerdict::deny(format!("path resolution failed: {e}")),
    };
    if let Err(v) = ensure_not_protected_or_root(&resolved, true) {
        return v;
    }

    match contains_subtree_protected_descendant(&resolved) {
        Ok(true) => ValidationVerdict::deny("path contains a subtree-protected descendant"),
        Ok(false) => ValidationVerdict::allow(),
        Err(e) => ValidationVerdict::deny(format!("failed to inspect descendants: {e}")),
    }
}

fn ensure_safe_custom_path_syntax(input: &str) -> Result<CanonicalPath, ValidationVerdict> {
    let lexical = CanonicalPath::parse_lexical(input)
        .map_err(|e| ValidationVerdict::deny(format!("invalid path: {e}")))?;
    if lexical.segment_count() < 3 {
        return Err(ValidationVerdict::deny(
            "custom path must have depth >= 3 segments",
        ));
    }
    Ok(lexical)
}

fn ensure_not_protected_or_root(
    path: &CanonicalPath,
    resolved: bool,
) -> Result<(), ValidationVerdict> {
    if resolved && path.segment_count() < 3 {
        return Err(ValidationVerdict::deny(
            "resolved path must have depth >= 3 segments",
        ));
    }
    if is_protected_for_selection(path) {
        let msg = if resolved {
            "resolved path is protected or within/above a protected location"
        } else {
            "path is protected or within/above a protected location"
        };
        return Err(ValidationVerdict::deny(msg));
    }
    Ok(())
}

/// Validation for Tier 3 conditional target roots (recycle-bin scope, stale project directories).
///
/// Stricter than "anything" but relaxed compared to Custom Path selection: allows non-protected
/// descendants under protected container roots when the root itself is not a protected container
/// root/subtree-protected path and contains no subtree-protected descendants and no link-point traversal.
/// This enables opt-in stale project cleanup in user dev folders while preserving safety invariants.
pub fn validate_tier3_target_root(input: &str) -> ValidationVerdict {
    let lexical = match CanonicalPath::parse_lexical(input) {
        Ok(p) => p,
        Err(e) => return ValidationVerdict::deny(format!("invalid path: {e}")),
    };
    if lexical.segment_count() < 3 {
        return ValidationVerdict::deny("tier 3 path must have depth >= 3 segments");
    }
    if is_subtree_protected(&lexical)
        || is_ancestor_of_subtree_protected(&lexical)
        || is_protected_container_root(&lexical)
    {
        return ValidationVerdict::deny("path is protected or within/above a protected location");
    }

    let validated = match CanonicalPath::from_existing_path(input) {
        Ok(v) => v,
        Err(PathError::NotFound) => {
            return ValidationVerdict::deny("path does not exist (no parent fallback)")
        }
        Err(PathError::LinkPoint) => return ValidationVerdict::deny("path is a link point"),
        Err(e) => return ValidationVerdict::deny(format!("path resolution failed: {e}")),
    };
    let resolved = validated.canonical();
    if resolved.segment_count() < 3 {
        return ValidationVerdict::deny("resolved path must have depth >= 3 segments");
    }
    if is_subtree_protected(resolved)
        || is_ancestor_of_subtree_protected(resolved)
        || is_protected_container_root(resolved)
    {
        return ValidationVerdict::deny(
            "resolved path is protected or within/above a protected location",
        );
    }
    match contains_subtree_protected_descendant(resolved) {
        Ok(true) => return ValidationVerdict::deny("path contains a subtree-protected descendant"),
        Ok(false) => {}
        Err(e) => return ValidationVerdict::deny(format!("failed to inspect descendants: {e}")),
    }
    ValidationVerdict::allow()
}

/// Returns true if `root` contains any subtree-protected descendant.
/// Walks without following link points; link points encountered are not traversed.
fn contains_subtree_protected_descendant(root: &CanonicalPath) -> Result<bool, String> {
    // Use iterative DFS over directory entries from winfs (read-only, does not follow link points).
    let mut stack: Vec<std::path::PathBuf> = vec![Path::new(root.as_str()).to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = sweeper_winfs::read_dir_entries(&dir).map_err(|e| e.to_string())?;
        for ent in entries {
            // Build a lexical path string for policy checks.
            let pstr = ent.path.to_string_lossy().to_string();
            if let Ok(cp) = CanonicalPath::parse_lexical(&pstr) {
                if is_subtree_protected(&cp) {
                    return Ok(true);
                }
                if ent.is_dir {
                    // Never traverse link points
                    if matches!(ent.reparse, ReparseTagClass::LinkPoint) {
                        continue;
                    }
                    // Also skip cloud placeholders and other reparse points conservatively
                    if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
                        continue;
                    }
                    stack.push(ent.path);
                }
            } else {
                // If a child path cannot be lexically normalized safely, be conservative only when it
                // obviously indicates protected names via raw component checks.
                if let Some(name) = ent.path.file_name().and_then(|s| s.to_str()) {
                    let u = name.to_ascii_uppercase();
                    if u == ".GIT"
                        || u == ".SSH"
                        || u == ".GNUPG"
                        || u == ".AWS"
                        || u == ".KUBE"
                        || u == ".DOCKER"
                        || u == "NTUSER.DAT"
                        || u.starts_with("NTUSER.DAT")
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

/// Public helper for analyzer/custom-path selection policy (lexical canonical path).
pub fn validate_selectable_path(path: &CanonicalPath) -> ValidationVerdict {
    if is_protected_for_selection(path) {
        ValidationVerdict::deny("path is protected or within/above a protected location")
    } else {
        ValidationVerdict::allow()
    }
}

/// Return a small canonical set of fixed protected examples (useful for tests).
pub fn sample_protected_paths() -> Vec<&'static str> {
    vec![
        r"C:\Windows\System32",
        r"C:\Windows\WinSxS",
        r"C:\Program Files",
        r"C:\Windows",
        r"C:\Users",
        r"C:\ProgramData",
        r"C:\",
    ]
}

/// Exposed for proptest: attempt many adversarial mutations logically by normalizing+checking.
/// (Actual property tests live in tests/; this helper keeps core API accessible.)
pub fn is_rejected_for_selection_input(input: &str) -> bool {
    match CanonicalPath::parse_lexical(input) {
        Ok(p) => is_protected_for_selection(&p),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::CanonicalPath;

    #[test]
    fn container_roots_blocked() {
        for s in [r"C:\", r"C:\Windows", r"C:\Users", r"C:\ProgramData"] {
            let p = if s == r"C:\" {
                paths::parse_drive_root(s).unwrap()
            } else {
                CanonicalPath::parse_lexical(s).unwrap()
            };
            assert!(is_protected_container_root(&p), "{s}");
            assert!(is_protected_for_selection(&p), "{s}");
        }
    }

    #[test]
    fn subtree_prefix_segment_safe() {
        let win = CanonicalPath::parse_lexical(r"C:\Windows").unwrap();
        let win_old = CanonicalPath::parse_lexical(r"C:\Windows.old").unwrap();
        assert!(is_protected_container_root(&win));
        assert!(!is_subtree_protected(&win_old));
        assert!(!is_protected_container_root(&win_old));
    }
}
