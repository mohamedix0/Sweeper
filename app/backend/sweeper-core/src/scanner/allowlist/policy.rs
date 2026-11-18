use super::*;
use std::path::Path;

pub(in crate::scanner) fn validate_resolved_canonical_root(
    root: &CanonicalPath,
) -> Result<(), String> {
    validate_resolved_root_depth(root)?;
    validate_resolved_root_policy(root)?;
    validate_resolved_root_links(root)
}

fn validate_resolved_root_depth(root: &CanonicalPath) -> Result<(), String> {
    if root.segment_count() < 3 {
        return Err("resolved target root must have depth >= 3 segments".to_string());
    }
    Ok(())
}

fn validate_resolved_root_policy(root: &CanonicalPath) -> Result<(), String> {
    if is_subtree_policy_violation(root) {
        return Err("resolved target root is subtree-protected".to_string());
    }
    if crate::policy::is_ancestor_of_subtree_protected(root) {
        return Err("resolved target root is ancestor of a subtree-protected path".to_string());
    }
    if crate::policy::is_protected_container_root(root) {
        return Err("resolved target root is a protected container root".to_string());
    }
    Ok(())
}

fn is_subtree_policy_violation(root: &CanonicalPath) -> bool {
    crate::policy::is_subtree_protected(root)
}

fn validate_resolved_root_links(root: &CanonicalPath) -> Result<(), String> {
    if is_link_point_self_root(root) {
        return Err("resolved target root is a link point".to_string());
    }
    if super::roots::path_has_link_point_ancestor(root)? {
        return Err("resolved target root traverses a link point ancestor".to_string());
    }
    Ok(())
}

fn is_link_point_self_root(root: &CanonicalPath) -> bool {
    sweeper_winfs::open_for_inspection(Path::new(root.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(false)
}

pub(super) fn validate_target_roots_policy(
    roots: &[(String, CanonicalPath)],
    all_existing: &[(String, CanonicalPath)],
) -> Result<(), String> {
    validate_roots_against_protected_policy(roots)?;
    validate_roots_no_overlap(roots, all_existing)
}

fn validate_roots_against_protected_policy(
    roots: &[(String, CanonicalPath)],
) -> Result<(), String> {
    for (_, r) in roots {
        validate_single_root_policy(r)?;
    }
    Ok(())
}

fn validate_single_root_policy(root: &CanonicalPath) -> Result<(), String> {
    if is_protected_or_ancestor(root) {
        return Err(format!(
            "target root not allowed by protected path policy: {root}"
        ));
    }
    if crate::policy::is_protected_container_root(root) {
        return Err(format!("target root is protected container root: {root}"));
    }
    Ok(())
}

fn is_protected_or_ancestor(root: &CanonicalPath) -> bool {
    crate::policy::is_subtree_protected(root)
        || crate::policy::is_ancestor_of_subtree_protected(root)
}

fn validate_roots_no_overlap(
    roots: &[(String, CanonicalPath)],
    all_existing: &[(String, CanonicalPath)],
) -> Result<(), String> {
    check_overlap_with_existing(roots, all_existing)?;
    check_overlap_within_roots(roots)
}

fn check_overlap_with_existing(
    roots: &[(String, CanonicalPath)],
    all_existing: &[(String, CanonicalPath)],
) -> Result<(), String> {
    for (_, a) in roots {
        for (_, b) in all_existing {
            reject_if_overlap(a, b)?;
        }
    }
    Ok(())
}

fn check_overlap_within_roots(roots: &[(String, CanonicalPath)]) -> Result<(), String> {
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            reject_if_overlap(&roots[i].1, &roots[j].1)?;
        }
    }
    Ok(())
}

fn reject_if_overlap(a: &CanonicalPath, b: &CanonicalPath) -> Result<(), String> {
    if paths_overlap(a, b) {
        return Err(format!(
            "overlap between resolved targets detected: {} and {}",
            a, b
        ));
    }
    Ok(())
}

pub(in crate::scanner) fn paths_overlap(a: &CanonicalPath, b: &CanonicalPath) -> bool {
    if a.is_same_path(b) {
        return true;
    }
    if a.is_self_or_ancestor_of(b) {
        return true;
    }
    b.is_self_or_ancestor_of(a)
}

// Single shared matcher: the scanner allowlist path and the Analyzer path both
// delegate to `crate::wildcard`, so their wildcard semantics cannot drift.
pub(super) use crate::wildcard::segment_pattern_matches;

#[cfg(test)]
mod tests {
    use super::segment_pattern_matches;

    /// F5 regression: a multi-byte directory name whose byte length satisfies
    /// the old guard but whose prefix slice offset is not a char boundary.
    #[test]
    fn multibyte_name_prefix_wildcard_returns_verdict_without_panic() {
        // "aéééé" is 9 bytes; "Profile " is 8 bytes -> old name[..8] panicked.
        assert!(!segment_pattern_matches(
            "a\u{e9}\u{e9}\u{e9}\u{e9}",
            "Profile *"
        ));
        assert!(segment_pattern_matches(
            "Profile \u{e9}\u{e9}\u{e9}",
            "PROFILE *"
        ));
    }
}
