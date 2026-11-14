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
            // Explicit equality case: selecting the protected path itself is blocked.
            if path.is_same_path(&prot) {
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
    if let Err(reason) = validate_resolved_tier3_root_policy(resolved) {
        return ValidationVerdict::deny(reason);
    }
    match contains_subtree_protected_descendant(resolved) {
        Ok(true) => return ValidationVerdict::deny("path contains a subtree-protected descendant"),
        Ok(false) => {}
        Err(e) => return ValidationVerdict::deny(format!("failed to inspect descendants: {e}")),
    }
    ValidationVerdict::allow()
}

/// Pure resolved-path policy check for Tier 3 target roots.
///
/// Kept separate so depth/protection operators are directly unit-testable without requiring
/// filesystem resolution to specific synthetic paths.
fn validate_resolved_tier3_root_policy(resolved: &CanonicalPath) -> Result<(), &'static str> {
    if resolved.segment_count() < 3 {
        return Err("resolved path must have depth >= 3 segments");
    }
    if is_subtree_protected(resolved)
        || is_ancestor_of_subtree_protected(resolved)
        || is_protected_container_root(resolved)
    {
        return Err("resolved path is protected and cannot be a Tier 3 Target");
    }
    Ok(())
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
                // If a child path cannot be lexically normalized safely, treat it as protected
                // to preserve safety (deny-by-default) rather than attempting heuristic parsing.
                return Ok(true);
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
    use crate::paths::{self, CanonicalPath};
    use std::fs;

    fn canon(s: &str) -> CanonicalPath {
        CanonicalPath::parse_lexical(s).expect("valid lexical path")
    }

    fn root(s: &str) -> CanonicalPath {
        paths::parse_drive_root(s).expect("valid drive root")
    }

    #[test]
    fn container_roots_blocked() {
        for s in [r"C:\", r"C:\Windows", r"C:\Users", r"C:\ProgramData"] {
            let p = if s == r"C:\" { root(s) } else { canon(s) };
            assert!(is_protected_container_root(&p), "{s}");
            assert!(is_protected_for_selection(&p), "{s}");
        }
    }

    #[test]
    fn subtree_prefix_segment_safe() {
        let win = canon(r"C:\Windows");
        let win_old = canon(r"C:\Windows.old");
        assert!(is_protected_container_root(&win));
        assert!(!is_subtree_protected(&win_old));
        assert!(!is_protected_container_root(&win_old));
    }

    #[test]
    fn is_drive_root_checks() {
        assert!(is_drive_root(&root(r"C:\")));
        assert!(!is_drive_root(&canon(r"C:\Temp")));
    }

    #[test]
    fn matches_fixed_subtree_protected_each_pattern() {
        for p in [
            r"C:\Users\Alice\.git",
            r"C:\Users\Alice\.ssh",
            r"C:\Users\Alice\.gnupg",
            r"C:\Users\Alice\.aws",
            r"C:\Users\Alice\.kube",
            r"C:\Users\Alice\.docker",
            r"C:\Recovery\foo",
            r"C:\Windows\System32\drivers",
            r"C:\Program Files\App",
            r"C:\Users\Alice\NTUSER.DAT",
            r"C:\Users\Alice\NTUSER.DAT.LOG1",
            r"C:\Users\Alice\ntuser.dat{12345678-1234-1234-1234-123456789abc}.TM.blf",
            r"C:\pagefile.sys",
        ] {
            assert!(is_subtree_protected(&canon(p)), "expected protected: {p}");
        }
        assert!(!is_subtree_protected(&canon(r"C:\Temp\safe")));
    }

    #[test]
    fn ancestor_of_subtree_protected_checks() {
        assert!(is_ancestor_of_subtree_protected(&root(r"C:\")));
        assert!(is_ancestor_of_subtree_protected(&canon(r"C:\Windows")));
        assert!(is_ancestor_of_subtree_protected(&canon(
            r"C:\Windows\System32"
        )));
        assert!(!is_ancestor_of_subtree_protected(&canon(r"C:\Temp\foo")));
    }

    #[test]
    fn path_last_segment_returns_exact_value() {
        assert_eq!(path_last_segment(&canon(r"C:\Temp\Leaf")), Some("Leaf"));
        assert_eq!(path_last_segment(&root(r"C:\")), None);
    }

    #[test]
    fn user_profile_root_checks() {
        assert!(is_user_profile_root(&canon(r"C:\Users\Alice")));
        assert!(!is_user_profile_root(&canon(r"C:\Users")));
        assert!(!is_user_profile_root(&canon(r"C:\Users\Alice\Documents")));
    }

    #[test]
    fn user_profile_data_subtree_checks() {
        // depth boundary: segs.len() < 3 false, ==3 allowed for data folders
        assert!(is_user_profile_data_subtree(&canon(
            r"C:\Users\Alice\Documents"
        )));
        assert!(is_user_profile_data_subtree(&canon(
            r"C:\Users\Alice\OneDrive - Contoso"
        )));
        assert!(!is_user_profile_data_subtree(&canon(r"C:\Users\Alice")));
        assert!(!is_user_profile_data_subtree(&canon(
            r"C:\Other\Alice\Documents"
        )));
        assert!(!is_user_profile_data_subtree(&canon(
            r"C:\Users\Alice\Projects"
        )));
    }

    #[test]
    fn matches_ci_any_checks() {
        assert!(matches_ci_any("Desktop", &["desktop", "music"]));
        assert!(!matches_ci_any("Projects", &["desktop", "music"]));
    }

    #[test]
    fn browser_container_root_checks() {
        // "User Data" leaf is always a browser container root (kills L258 replacement/early-return mutants)
        assert!(is_browser_container_root(&canon(
            r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data"
        )));

        // "Default"/"Profile *" only when under a "User Data" path (kills L267 == vs !=, L273 && vs ||)
        assert!(is_browser_container_root(&canon(
            r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data\Default"
        )));
        assert!(is_browser_container_root(&canon(
            r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data\Profile 1"
        )));
        assert!(!is_browser_container_root(&canon(
            r"C:\Users\Alice\AppData\Local\Default"
        )));
        assert!(!is_browser_container_root(&canon(
            r"C:\Users\Alice\AppData\Local\Profile 1"
        )));

        // Negative cases
        assert!(!is_browser_container_root(&canon(
            r"C:\Users\Alice\AppData\Local\Temp"
        )));
    }

    #[test]
    fn ensure_safe_custom_path_syntax_depth_boundary() {
        let ok = canon(r"C:\A\B\C");
        let bad = canon(r"C:\A\B");
        assert!(ensure_safe_custom_path_syntax(ok.as_str()).is_ok());
        let err = ensure_safe_custom_path_syntax(bad.as_str()).unwrap_err();
        assert!(!err.allowed);
        assert!(err.reason.contains("depth >= 3"));
    }

    #[test]
    fn ensure_not_protected_or_root_depth_and_protection() {
        let depth2 = CanonicalPath::parse_lexical(r"C:\Foo").unwrap();
        let depth3 = CanonicalPath::parse_lexical(r"C:\Sandbox\Foo\Leaf").unwrap();

        // Depth 2 resolved=true must be denied by resolved-depth guard
        // (kills -> Ok(()), < -> ==, < -> <=)
        let resolved_depth2 = ensure_not_protected_or_root(&depth2, true).unwrap_err();
        assert!(!resolved_depth2.allowed);
        assert!(
            resolved_depth2
                .reason
                .contains("resolved path must have depth >= 3"),
            "unexpected reason: {}",
            resolved_depth2.reason
        );

        // Depth 2 resolved=false is not rejected by the resolved-depth guard (guard is resolved-only);
        // safe non-protected lexical paths remain allowed here.
        assert!(ensure_not_protected_or_root(&depth2, false).is_ok());

        // Depth 3 resolved=true must be allowed (kills < -> <=)
        assert!(ensure_not_protected_or_root(&depth3, true).is_ok());

        let protected = canon(r"C:\Windows\System32\drivers");
        let denied = ensure_not_protected_or_root(&protected, false).unwrap_err();
        assert!(!denied.allowed);
        assert!(
            denied
                .reason
                .contains("path is protected or within/above a protected location"),
            "unexpected reason: {}",
            denied.reason
        );
    }

    #[test]
    fn validate_tier3_target_root_branches() {
        // Lexical depth boundary (L401): depth 2 denied with depth reason before existence checks
        let depth2 = validate_tier3_target_root(r"C:\A\B");
        assert!(!depth2.allowed, "depth 2 should be denied");
        assert!(
            depth2.reason.contains("depth >= 3"),
            "expected depth reason, got: {}",
            depth2.reason
        );
        assert!(
            !depth2.reason.contains("does not exist"),
            "lexical depth check must not fall through to from_existing_path: {}",
            depth2.reason
        );

        // Lexical depth 3 missing path reaches existence check (kills L401 < -> <=)
        let missing_depth3 = validate_tier3_target_root(r"C:\NonExistentDepthThree\Sub\Leaf");
        assert!(!missing_depth3.allowed, "missing depth 3 should be denied");
        assert!(
            missing_depth3.reason.contains("does not exist"),
            "expected existence denial, got: {}",
            missing_depth3.reason
        );
        assert!(
            !missing_depth3.reason.contains("depth >= 3"),
            "depth 3 must not be rejected by lexical depth check: {}",
            missing_depth3.reason
        );

        // Lexical protected checks (L405-406, || -> &&) must deny before from_existing_path.
        // Subtree-protected path (depth >= 3), missing on disk.
        let missing_subtree_protected =
            validate_tier3_target_root(r"C:\Windows\System32\NonExistentTier3Subdir");
        assert!(!missing_subtree_protected.allowed);
        assert!(
            missing_subtree_protected
                .reason
                .contains("path is protected or within/above a protected location"),
            "expected protected lexical denial, got: {}",
            missing_subtree_protected.reason
        );
        assert!(
            !missing_subtree_protected.reason.contains("does not exist"),
            "subtree-protected lexical check must run before from_existing_path: {}",
            missing_subtree_protected.reason
        );

        // Protected container root by shape (browser User Data, depth >= 3), missing on disk.
        let missing_container_root = validate_tier3_target_root(
            r"C:\NonExistentBrowser\AppData\Local\Vendor\Browser\User Data",
        );
        assert!(!missing_container_root.allowed);
        assert!(
            missing_container_root
                .reason
                .contains("path is protected or within/above a protected location"),
            "expected protected container root lexical denial, got: {}",
            missing_container_root.reason
        );
        assert!(
            !missing_container_root.reason.contains("does not exist"),
            "container-root lexical check must run before from_existing_path: {}",
            missing_container_root.reason
        );

        // Other lexical protections / existing paths
        let drivers = validate_tier3_target_root(r"C:\Windows\System32\drivers");
        assert!(!drivers.allowed);
        assert!(
            drivers
                .reason
                .contains("path is protected or within/above a protected location"),
            "unexpected reason: {}",
            drivers.reason
        );

        let windows_root = validate_tier3_target_root(r"C:\Windows");
        assert!(!windows_root.allowed);
        assert!(
            windows_root.reason.contains("depth >= 3") || windows_root.reason.contains("protected")
        );

        let users_root = validate_tier3_target_root(r"C:\Users");
        assert!(!users_root.allowed);
        assert!(
            users_root.reason.contains("depth >= 3") || users_root.reason.contains("protected")
        );

        let sandbox = sweeper_testkit::Sandbox::new().expect("sandbox");
        // clean depth >= 3 directory allowed
        let clean = sandbox.create_dir("a/b/c").expect("create clean dir");
        let clean_str = clean.to_string_lossy().to_string();
        let verdict = validate_tier3_target_root(&clean_str);
        assert!(
            verdict.allowed,
            "expected clean tier3 root allowed: {} ({})",
            verdict.reason, clean_str
        );

        // directory containing protected descendant denied
        let dirty_root = sandbox.create_dir("x/y/z").expect("create dirty root");
        let git_dir = dirty_root.join(".git");
        fs::create_dir_all(&git_dir).expect("create .git");
        let dirty_str = dirty_root.to_string_lossy().to_string();
        let denied = validate_tier3_target_root(&dirty_str);
        assert!(!denied.allowed, "expected protected descendant denial");
        assert!(
            denied.reason.contains("protected descendant"),
            "unexpected reason: {}",
            denied.reason
        );

        // missing sandbox path denied (no parent fallback) with existence reason
        let missing = sandbox.guarded_path("missing/a/b");
        let missing_sandbox = validate_tier3_target_root(&missing.to_string_lossy());
        assert!(!missing_sandbox.allowed);
        assert!(
            missing_sandbox.reason.contains("does not exist"),
            "unexpected reason: {}",
            missing_sandbox.reason
        );
    }

    #[test]
    fn contains_subtree_protected_descendant_detects_each_pattern() {
        let patterns = [
            ".git",
            ".ssh",
            ".gnupg",
            ".aws",
            ".kube",
            ".docker",
            "NTUSER.DAT",
        ];
        for pat in patterns {
            let sb = sweeper_testkit::Sandbox::new().expect("sandbox");
            let root_dir = sb.create_dir("a/b/c").expect("root dir");
            let target = root_dir.join(pat);
            if pat == "NTUSER.DAT" {
                fs::write(&target, b"x").expect("write file");
            } else {
                fs::create_dir_all(&target).expect("create dir");
            }
            let cp = CanonicalPath::parse_lexical(&root_dir.to_string_lossy()).expect("canonical");
            assert!(
                contains_subtree_protected_descendant(&cp).expect("scan ok"),
                "expected descendant {pat} to be detected"
            );
        }

        let clean_sb = sweeper_testkit::Sandbox::new().expect("sandbox");
        let clean_root = clean_sb.create_dir("a/b/c").expect("clean root");
        let clean_cp =
            CanonicalPath::parse_lexical(&clean_root.to_string_lossy()).expect("canonical");
        assert!(!contains_subtree_protected_descendant(&clean_cp).expect("scan ok"));
    }

    #[test]
    fn contains_subtree_protected_descendant_traverses_normal_dirs_and_finds_nested() {
        // Kills mutant where traversal/recursion into normal directories is skipped.
        let sb = sweeper_testkit::Sandbox::new().expect("sandbox");
        let root_dir = sb.create_dir("a/b/c").expect("root dir");
        let nested = root_dir.join("normal_subdir").join("deeper");
        fs::create_dir_all(&nested).expect("create nested dirs");
        fs::create_dir_all(nested.join(".git")).expect("create nested .git");
        let cp = CanonicalPath::parse_lexical(&root_dir.to_string_lossy()).expect("canonical");
        assert!(
            contains_subtree_protected_descendant(&cp).expect("scan ok"),
            "must detect protected descendant nested under normal directories"
        );
    }

    #[test]
    fn validate_resolved_tier3_root_policy_depth_boundaries_and_protection() {
        // Depth boundaries (segment_count after drive root): 2 => Err, 3 => Ok when not protected.
        let depth2 = canon(r"C:\Temp\Sub");
        assert_eq!(depth2.segment_count(), 2);
        assert_eq!(
            validate_resolved_tier3_root_policy(&depth2),
            Err("resolved path must have depth >= 3 segments")
        );

        let depth3 = canon(r"C:\Temp\Sub\Leaf");
        assert_eq!(depth3.segment_count(), 3);
        assert!(validate_resolved_tier3_root_policy(&depth3).is_ok());

        // Subtree-protected only: segment-named secret under user profile.
        let subtree_only = canon(r"C:\Users\Alice\.ssh");
        assert!(is_subtree_protected(&subtree_only));
        assert!(!is_protected_container_root(&subtree_only));
        assert_eq!(
            validate_resolved_tier3_root_policy(&subtree_only),
            Err("resolved path is protected and cannot be a Tier 3 Target")
        );

        // Protected container root only (browser User Data shape): not subtree-protected.
        let container_only = canon(r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data");
        assert!(is_protected_container_root(&container_only));
        assert!(!is_subtree_protected(&container_only));
        assert_eq!(
            validate_resolved_tier3_root_policy(&container_only),
            Err("resolved path is protected and cannot be a Tier 3 Target")
        );

        // Ancestor-of-fixed-subtree-protected also applies to some paths (e.g. C:\Windows is an
        // ancestor of fixed subtree-protected prefixes), but depth/container protections may also
        // apply depending on the path. Depth guard is evaluated first for safety.
        let windows_root = canon(r"C:\Windows");
        assert!(is_protected_container_root(&windows_root));
        assert!(is_ancestor_of_subtree_protected(&windows_root));
        assert_eq!(
            validate_resolved_tier3_root_policy(&windows_root),
            Err("resolved path must have depth >= 3 segments")
        );

        let drive_root = root(r"C:\");
        assert_eq!(
            validate_resolved_tier3_root_policy(&drive_root),
            Err("resolved path must have depth >= 3 segments")
        );
    }

    #[test]
    fn validate_tier3_target_root_resolved_protection_on_existing_paths() {
        // Keep end-to-end validation on real tempdir paths.
        let sb = sweeper_testkit::Sandbox::new().expect("sandbox");

        let clean = sb.create_dir("tier3/clean/leaf").expect("create clean dir");
        let clean_v = validate_tier3_target_root(&clean.to_string_lossy());
        assert!(
            clean_v.allowed,
            "clean path should be allowed: {}",
            clean_v.reason
        );

        let ssh_root = sb.create_dir("tier3/prot/.ssh").expect("create .ssh root");
        let ssh_v = validate_tier3_target_root(&ssh_root.to_string_lossy());
        assert!(!ssh_v.allowed, "subtree-protected root must be denied");

        let user_data_root = sb
            .create_dir("tier3/prot/AppData/Local/Vendor/Browser/User Data")
            .expect("create User Data root");
        let ud_v = validate_tier3_target_root(&user_data_root.to_string_lossy());
        assert!(!ud_v.allowed, "container root must be denied");

        let profile_root = sb
            .create_dir("tier3/prot/AppData/Local/Vendor/Browser/User Data/Default")
            .expect("create profile root");
        let p_v = validate_tier3_target_root(&profile_root.to_string_lossy());
        assert!(!p_v.allowed, "profile container root must be denied");

        let dirty_root = sb
            .create_dir("tier3/dirty/root")
            .expect("create dirty root");
        fs::create_dir_all(dirty_root.join("nested").join(".git")).expect("create nested .git");
        let dirty_v = validate_tier3_target_root(&dirty_root.to_string_lossy());
        assert!(
            !dirty_v.allowed,
            "path containing protected descendant must be denied"
        );
        assert!(dirty_v.reason.contains("protected descendant"));
    }

    #[test]
    fn sample_protected_paths_is_meaningful() {
        let samples = sample_protected_paths();
        assert!(!samples.is_empty(), "sample list must not be empty");
        assert!(
            samples.iter().all(|s| !s.is_empty()),
            "sample list must not contain empty strings"
        );
        assert!(samples.contains(&r"C:\Windows"), "must include C:\\Windows");
        assert!(
            samples.contains(&r"C:\Windows\System32"),
            "must include C:\\Windows\\System32"
        );
        assert!(samples.contains(&r"C:\"));
    }

    #[test]
    fn is_rejected_for_selection_input_checks() {
        assert!(is_rejected_for_selection_input(r"C:\Windows\System32"));
        assert!(is_rejected_for_selection_input(r"C:\Windows"));
        assert!(is_rejected_for_selection_input(r"not-a-path"));
        assert!(!is_rejected_for_selection_input(r"C:\Sandbox\Cache"));
        assert!(!is_rejected_for_selection_input(r"C:\Temp\SafeArea\Cache"));
    }
}
