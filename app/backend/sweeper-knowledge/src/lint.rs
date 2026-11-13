//! Build-time/runtime linter for the embedded Sweeper allowlist.
//!
//! Enforces safety invariants required for `allowlist.toml` (deny-by-default, no overlap with
//! protected paths, constrained wildcards, and no ambiguous path forms).

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use thiserror::Error;

/// Lint/validation errors for an allowlist definition.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LintError {
    /// TOML parsing failed.
    #[error("failed to parse allowlist toml: {0}")]
    Parse(String),
    /// A target failed validation.
    #[error("target `{id}`: {message}")]
    Target {
        /// Target id (or `<unknown>`).
        id: String,
        /// Validation failure detail.
        message: String,
    },
    /// Duplicate target id.
    #[error("duplicate target id `{0}`")]
    DuplicateId(String),
    /// Duplicate target path (known folder + relative subpath).
    #[error("duplicate target path for known_folder={known_folder:?} relative_subpath={relative_subpath}")]
    DuplicatePath {
        /// Known folder.
        known_folder: String,
        /// Relative subpath.
        relative_subpath: String,
    },
    /// Overlapping targets.
    #[error("overlapping targets `{a}` and `{b}`: {reason}")]
    Overlap {
        /// First target id.
        a: String,
        /// Second target id.
        b: String,
        /// Reason/detail.
        reason: String,
    },
}

#[derive(Debug, Deserialize, Clone)]
struct AllowlistFile {
    #[serde(default)]
    targets: Vec<LintTarget>,
}

/// Minimal target shape used by the linter/build-time checks.
/// Mirrors `crate::TargetDefinition` fields relevant to validation.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
pub struct LintTarget {
    /// Stable target identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Tier (string form for tolerant parsing in build/lint; validated semantically).
    pub tier: String,
    /// Risk (string form).
    pub risk: String,
    /// Known folder root.
    pub known_folder: String,
    /// Relative subpath under the known folder.
    pub relative_subpath: String,
    /// Scope (contents-only invariant expected).
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Minimum age in hours.
    #[serde(default = "default_min_age_hours")]
    pub min_age_hours: u32,
    /// Description for Knowledge Base/UI.
    #[serde(default)]
    pub description: String,
    /// Whether elevation is required.
    #[serde(default)]
    pub requires_elevation: bool,
}

fn default_scope() -> String {
    "contents".to_string()
}
fn default_min_age_hours() -> u32 {
    24
}

/// Parse and lint allowlist TOML text.
///
/// This is the canonical linter used by `build.rs` and library tests.
pub fn lint_allowlist_toml(toml_text: &str) -> Result<Vec<LintTarget>, LintError> {
    let parsed: AllowlistFile =
        toml::from_str(toml_text).map_err(|e| LintError::Parse(e.to_string()))?;
    lint_targets(&parsed.targets)?;
    Ok(parsed.targets)
}

/// Lint a set of targets against Sweeper allowlist safety rules.
pub fn lint_targets(targets: &[LintTarget]) -> Result<(), LintError> {
    // Duplicate ID / duplicate path
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_paths: HashMap<(String, String), String> = HashMap::new();
    for t in targets {
        if !seen_ids.insert(t.id.clone()) {
            return Err(LintError::DuplicateId(t.id.clone()));
        }
        let key = (t.known_folder.clone(), t.relative_subpath.clone());
        if let Some(prev) = seen_paths.insert(key.clone(), t.id.clone()) {
            let _ = prev;
            return Err(LintError::DuplicatePath {
                known_folder: key.0,
                relative_subpath: key.1,
            });
        }
    }

    // Per-target structural/safety validation
    for t in targets {
        validate_target(t)?;
    }

    // Pairwise overlap checks (including wildcard segment overlap)
    for i in 0..targets.len() {
        for j in (i + 1)..targets.len() {
            if targets_overlap(&targets[i], &targets[j]) {
                return Err(LintError::Overlap {
                    a: targets[i].id.clone(),
                    b: targets[j].id.clone(),
                    reason:
                        "same path or ancestor/descendant relationship (including wildcard overlap)"
                            .to_string(),
                });
            }
        }
    }

    Ok(())
}

fn terr(id: &str, msg: impl Into<String>) -> LintError {
    LintError::Target {
        id: id.to_string(),
        message: msg.into(),
    }
}

fn validate_target(t: &LintTarget) -> Result<(), LintError> {
    if t.id.trim().is_empty() {
        return Err(terr("<empty>", "id must not be empty"));
    }
    if t.name.trim().is_empty() {
        return Err(terr(&t.id, "name must not be empty"));
    }
    // Tier/risk/scope basic sanity (embedded allowlist is controlled, but linter must be strict)
    match t.tier.as_str() {
        "Tier1" | "Tier2" | "Tier3" => {}
        _ => return Err(terr(&t.id, "tier must be Tier1, Tier2, or Tier3")),
    }
    match t.risk.as_str() {
        "Safe" | "Caution" | "Unknown" | "Protected" => {}
        _ => {
            return Err(terr(
                &t.id,
                "risk must be Safe, Caution, Unknown, or Protected",
            ))
        }
    }
    if t.scope != "contents" {
        return Err(terr(&t.id, "scope must be \"contents\""));
    }
    if t.requires_elevation {
        // Phase 1B Tier 1/2 targets must not require elevation.
        return Err(terr(
            &t.id,
            "requires_elevation must be false for Tier 1 & 2 allowlist targets",
        ));
    }
    validate_known_folder(&t.id, &t.known_folder)?;
    let segments = validate_relative_subpath(&t.id, &t.relative_subpath)?;

    // Reject direct targeting of Protected Container Roots
    if is_protected_container_root_target(&t.known_folder, &segments) {
        return Err(terr(
            &t.id,
            "target directly targets a Protected Container Root",
        ));
    }

    // Reject overlap with any Subtree-Protected Path (ancestor, exact, or descendant)
    if overlaps_subtree_protected(&t.known_folder, &segments) {
        return Err(terr(
            &t.id,
            "target overlaps a Subtree-Protected Path (ancestor, exact, or descendant)",
        ));
    }

    // Browser profile roots must not be targeted directly without a leaf cache subfolder
    validate_browser_profile_leaf_rule(&t.id, &segments)?;

    Ok(())
}

fn validate_known_folder(id: &str, kf: &str) -> Result<(), LintError> {
    match kf {
        "LocalAppData" | "RoamingAppData" | "UserProfile" | "Windows" | "ProgramData" => Ok(()),
        _ => Err(terr(
            id,
            format!("unknown or unsupported known_folder `{kf}`"),
        )),
    }
}

fn validate_relative_subpath(id: &str, s: &str) -> Result<Vec<String>, LintError> {
    if s.is_empty() {
        return Err(terr(id, "relative_subpath must not be empty"));
    }
    if s.contains("**") {
        return Err(terr(id, "glob `**` is not allowed"));
    }
    // Reject backslash/slash ambiguity and require '\' separators only.
    if s.contains('/') {
        return Err(terr(
            id,
            "relative_subpath must use '\\' separators (backslash/slash ambiguity not allowed)",
        ));
    }
    if s.contains(':') {
        return Err(terr(
            id,
            "relative_subpath must not contain drive letters or ':'",
        ));
    }
    if s.starts_with('\\') || s.starts_with("\\\\") {
        return Err(terr(
            id,
            "relative_subpath must be relative (no absolute prefix)",
        ));
    }

    let raw_segments: Vec<&str> = s.split('\\').collect();
    if raw_segments.is_empty() {
        return Err(terr(id, "relative_subpath must not be empty"));
    }

    let mut segments: Vec<String> = Vec::with_capacity(raw_segments.len());
    let mut wildcard_segment_count = 0usize;
    for (i, seg) in raw_segments.iter().enumerate() {
        if seg.is_empty() {
            return Err(terr(id, "relative_subpath contains an empty segment"));
        }
        if *seg == "." || *seg == ".." {
            return Err(terr(
                id,
                "relative_subpath must not contain '.' or '..' segments",
            ));
        }
        if seg.ends_with(' ') || seg.ends_with('.') {
            return Err(terr(id, "path segments must not end with space or dot"));
        }
        let star_count = seg.chars().filter(|c| *c == '*').count();
        if star_count > 0 {
            wildcard_segment_count += 1;
            // Disallow partial/ambiguous forms beyond a single '*' character in the segment pattern.
            // (e.g. reject multiple '*' in same segment; '**' already rejected above)
            if star_count != 1 {
                return Err(terr(id, "ambiguous wildcard usage in segment"));
            }
            let is_last = i + 1 == raw_segments.len();
            if is_last {
                return Err(terr(
                    id,
                    "wildcard '*' is not allowed in the final path segment",
                ));
            }
        }
        segments.push((*seg).to_string());
    }
    if wildcard_segment_count > 1 {
        return Err(terr(
            id,
            "at most one intermediate wildcard segment ('*') is allowed in relative_subpath",
        ));
    }

    Ok(segments)
}

/// Protected container root targets (exact identities) that must not be targeted directly.
/// Modeled relative to known folders used by the allowlist.
fn is_protected_container_root_target(kf: &str, segments: &[String]) -> bool {
    match kf {
        "Windows" => segments.is_empty(),     // C:\Windows root
        "ProgramData" => segments.is_empty(), // C:\ProgramData root
        "UserProfile" => segments.is_empty(), // C:\Users\<name> profile root
        _ => false,
    }
    // Note: Drive roots and C:\Users root are not representable via current KnownFolder enum targets.
}

/// Returns true if the target path (known folder + relative segments) is ancestor/equal/descendant
/// of any Subtree-Protected Path defined in docs/IMPLEMENTATION_PLAN.md §3.4.
fn overlaps_subtree_protected(kf: &str, segments: &[String]) -> bool {
    // Represent target as a path-kind + segments under a fixed root namespace for comparison.
    // For each subtree-protected prefix, test ancestor/exact/descendant relation in same namespace.
    for prot in subtree_protected_specs() {
        if path_relation(kf, segments, &prot) != PathRelation::Disjoint {
            return true;
        }
    }
    false
}

fn subtree_protected_specs() -> Vec<(ProtKind, Vec<SegPat>)> {
    // Encode fixed subtree-protected paths as concrete segment sequences (no wildcards).
    // Patterns like "any directory named .git" and "NTUSER.DAT*" are handled specially.
    vec![
        (ProtKind::Windows, segs(&["System32"])),
        (ProtKind::Windows, segs(&["WinSxS"])),
        (ProtKind::Windows, segs(&["SysWOW64"])),
        (ProtKind::Windows, segs(&["Boot"])),
        (ProtKind::Windows, segs(&["Fonts"])),
        (ProtKind::Windows, segs(&["servicing"])),
        (ProtKind::ProgramFiles, vec![]),
        (ProtKind::ProgramFilesX86, vec![]),
        // User data folders under user profile
        (ProtKind::UserProfile, segs(&["Desktop"])),
        (ProtKind::UserProfile, segs(&["Documents"])),
        (ProtKind::UserProfile, segs(&["Downloads"])),
        (ProtKind::UserProfile, segs(&["Pictures"])),
        (ProtKind::UserProfile, segs(&["Videos"])),
        (ProtKind::UserProfile, segs(&["Music"])),
        // OneDrive sync roots (common) - treat as subtree-protected by prefix "OneDrive"
        (ProtKind::UserProfileOneDrivePrefix, vec![]),
        // Secrets & VCS fixed dirs/files in profile
        (ProtKind::UserProfile, segs(&[".ssh"])),
        (ProtKind::UserProfile, segs(&[".gnupg"])),
        (ProtKind::UserProfile, segs(&[".aws"])),
        (ProtKind::UserProfile, segs(&[".kube"])),
        (ProtKind::UserProfile, segs(&[".docker"])),
        // OS internals in profile/roots (NTUSER.DAT* treated specially)
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProtKind {
    Windows,
    ProgramFiles,
    ProgramFilesX86,
    UserProfile,
    UserProfileOneDrivePrefix,
}

#[derive(Clone, PartialEq, Eq)]
enum SegPat {
    Lit(String),
}

fn segs(items: &[&str]) -> Vec<SegPat> {
    items
        .iter()
        .map(|s| SegPat::Lit((*s).to_string()))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathRelation {
    Disjoint,
    Same,
    TargetContainsProtected, // target is ancestor of protected
    ProtectedContainsTarget, // protected is ancestor of target (or exact handled above)
}

fn path_relation(kf: &str, target: &[String], prot: &(ProtKind, Vec<SegPat>)) -> PathRelation {
    // Special: any directory named .git is subtree-protected anywhere. If target path includes
    // a literal ".git" segment, it overlaps (exact/descendant); if not, we cannot prove ancestor-of-.git
    // lexically except not applicable. (Targets with wildcards are not expected to create .git names.)
    if target.iter().any(|s| s.eq_ignore_ascii_case(".git")) {
        return PathRelation::ProtectedContainsTarget;
    }

    match prot.0 {
        ProtKind::Windows => {
            if kf != "Windows" {
                return PathRelation::Disjoint;
            }
            rel_concrete_relation(target, &prot.1)
        }
        ProtKind::ProgramFiles => {
            // Not representable as a target under supported KnownFolder enum values (Program Files is not one).
            // But reject if someone tried to express equivalent via unsupported folder (already rejected).
            let _ = target;
            PathRelation::Disjoint
        }
        ProtKind::ProgramFilesX86 => {
            let _ = target;
            PathRelation::Disjoint
        }
        ProtKind::UserProfile => {
            if kf != "UserProfile" {
                return PathRelation::Disjoint;
            }
            rel_concrete_relation(target, &prot.1)
        }
        ProtKind::UserProfileOneDrivePrefix => {
            if kf != "UserProfile" || target.is_empty() {
                return PathRelation::Disjoint;
            }
            let first = target[0].to_ascii_uppercase();
            let is_onedrive = first == "ONEDRIVE"
                || first.starts_with("ONEDRIVE -")
                || first.starts_with("ONEDRIVE-");
            if !is_onedrive {
                return PathRelation::Disjoint;
            }
            // Protected is OneDrive root subtree (conceptually first segment prefix). Target equal/descendant/ancestor.
            // Treat protected as the profile-relative path [<OneDrive*>] root; since target's first matches prefix,
            // consider overlap: if target has only that segment (possibly exact name) it's same/within; if deeper it's descendant.
            // But our target first may be literal "OneDrive" not wildcard; still overlaps.
            if target.len() == 1 {
                PathRelation::Same
            } else {
                // target is inside OneDrive subtree (or deeper) => protected contains target
                PathRelation::ProtectedContainsTarget
            }
        }
    }
}

fn rel_concrete_relation(target: &[String], prot: &[SegPat]) -> PathRelation {
    // Compare segment sequences with wildcard matching in target segments (intermediate only).
    // For relation against a concrete protected path:
    let tlen = target.len();
    let plen = prot.len();
    if tlen == 0 && plen == 0 {
        return PathRelation::Same;
    }
    // Determine if sequences are comparable (same concrete prefix where both specified)
    let min = tlen.min(plen);
    for i in 0..min {
        let t = &target[i];
        let p = match &prot[i] {
            SegPat::Lit(s) => s.as_str(),
        };
        if !segment_pattern_matches_literal(t, p) {
            return PathRelation::Disjoint;
        }
    }
    if tlen == plen {
        PathRelation::Same
    } else if tlen < plen {
        // target is ancestor of protected path
        PathRelation::TargetContainsProtected
    } else {
        // protected is ancestor of target
        PathRelation::ProtectedContainsTarget
    }
}

/// Match an allowlist segment pattern (may include a single '*' wildcard, except final handled earlier)
/// against a concrete literal segment.
fn segment_pattern_matches_literal(pattern: &str, literal: &str) -> bool {
    if !pattern.contains('*') {
        return pattern.eq_ignore_ascii_case(literal);
    }
    // Single '*' wildcard in segment: glob-style match for that segment.
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return false; // ambiguity rejected elsewhere
    }
    let (pre, suf) = (parts[0], parts[1]);
    if literal.len() < pre.len() + suf.len() {
        return false;
    }
    literal[..pre.len()].eq_ignore_ascii_case(pre)
        && literal[literal.len() - suf.len()..].eq_ignore_ascii_case(suf)
}

fn validate_browser_profile_leaf_rule(id: &str, segments: &[String]) -> Result<(), LintError> {
    // Reject browser profile roots targeted directly without a leaf cache subfolder.
    // Applies to Chromium-style "...\\User Data\\Default" and "...\\User Data\\Profile *" roots.
    if segments.len() < 2 {
        return Ok(());
    }
    let leaf_caches = [
        "cache",
        "code cache",
        "gpucache",
        "shadercache",
        "grshadercache",
        "dawncache",
    ];
    let last = segments[segments.len() - 1].to_ascii_lowercase();
    let is_leaf_cache = leaf_caches.contains(&last.as_str());

    // Find if path includes "User Data" followed by "Default" or "Profile *" as a profile root.
    for i in 0..segments.len() {
        if segments[i].eq_ignore_ascii_case("User Data") && i + 1 < segments.len() {
            let prof = &segments[i + 1];
            let is_profile_root = prof.eq_ignore_ascii_case("Default") || is_profile_star(prof);
            if is_profile_root {
                // If target ends at profile root (i+1 is last) or at User Data root, reject.
                if i + 1 == segments.len() - 1 {
                    return Err(terr(
                        id,
                        "browser profile root targeted directly; must specify an explicit leaf cache subfolder",
                    ));
                }
                // Also reject targeting "User Data" itself
                // Require that when under a browser profile root, the final segment is an explicit leaf cache subfolder.
                if !is_leaf_cache {
                    return Err(terr(
                        id,
                        "browser profile path must target an explicit leaf cache subfolder (e.g. Cache, Code Cache, GPUCache)",
                    ));
                }
                return Ok(());
            }
        }
    }
    // If targeting "User Data" directly (ends with it)
    if segments
        .last()
        .map(|s| s.eq_ignore_ascii_case("User Data"))
        .unwrap_or(false)
    {
        return Err(terr(
            id,
            "browser User Data root targeted directly without a leaf cache subfolder",
        ));
    }
    Ok(())
}

fn is_profile_star(seg: &str) -> bool {
    // Accept "Profile *" style wildcard segment.
    if !seg.contains('*') {
        return false;
    }
    let upper = seg.to_ascii_uppercase();
    upper.starts_with("PROFILE ") && seg.matches('*').count() == 1 && seg.ends_with('*')
}

/// Determine if two targets overlap (same path or ancestor/descendant), including wildcard segment overlap.
fn targets_overlap(a: &LintTarget, b: &LintTarget) -> bool {
    if a.known_folder != b.known_folder {
        return false;
    }
    let sa: Vec<&str> = a.relative_subpath.split('\\').collect();
    let sb: Vec<&str> = b.relative_subpath.split('\\').collect();
    // Overlap if one sequence can match a prefix-equivalent of the other considering wildcards in either.
    // i.e. there exists some concrete path matched by both, and one target path is ancestor-or-equal of that concrete path
    // relative to specificity? For allowlist targets, any possible concrete intersection with ancestor/descendant relation is unsafe overlap.
    paths_overlap(&sa, &sb)
}

fn paths_overlap(a: &[&str], b: &[&str]) -> bool {
    let min = a.len().min(b.len());
    for i in 0..min {
        if !segments_may_match(a[i], b[i]) {
            return false;
        }
    }
    // If all compared segments may match, then the shorter is ancestor-or-equal of any concrete extension
    // of the longer that matches -> overlap.
    true
}

fn segments_may_match(a: &str, b: &str) -> bool {
    // Both may be patterns (single '*') or literals.
    match (a.contains('*'), b.contains('*')) {
        (false, false) => a.eq_ignore_ascii_case(b),
        (true, false) => segment_pattern_matches_literal(a, b),
        (false, true) => segment_pattern_matches_literal(b, a),
        (true, true) => wildcard_patterns_may_intersect(a, b),
    }
}

fn wildcard_patterns_may_intersect(a: &str, b: &str) -> bool {
    // Conservative check for single-'*' segment patterns: try to find if any literal could satisfy both.
    // Handle common forms: "*", "prefix*", "*suffix", "prefix*suffix", and "Profile *".
    let pa = parse_star_pattern(a);
    let pb = parse_star_pattern(b);
    match (pa, pb) {
        (Some((ap, asuf)), Some((bp, bsuf))) => {
            // prefixes must be compatible (one prefix of other case-insensitively or equal)
            let (short_p, long_p) = if ap.len() <= bp.len() {
                (ap, bp)
            } else {
                (bp, ap)
            };
            if !long_p[..short_p.len()].eq_ignore_ascii_case(short_p) {
                return false;
            }
            let (short_s, long_s) = if asuf.len() <= bsuf.len() {
                (asuf, bsuf)
            } else {
                (bsuf, asuf)
            };
            if !long_s[long_s.len() - short_s.len()..].eq_ignore_ascii_case(short_s) {
                return false;
            }
            // Possible intersection exists (e.g. satisfy longer prefix/suffix constraints).
            true
        }
        _ => true, // be conservative for unexpected forms
    }
}

fn parse_star_pattern(s: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = s.split('*').collect();
    if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
}
