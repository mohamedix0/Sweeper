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

pub(super) fn segment_pattern_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return name.eq_ignore_ascii_case(pattern);
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return false;
    }
    wildcard_segment_matches(name, parts[0], parts[1])
}

fn wildcard_segment_matches(name: &str, prefix: &str, suffix: &str) -> bool {
    let name_len = name.len();
    let pre_len = prefix.len();
    let suf_len = suffix.len();
    if name_len < pre_len + suf_len {
        return false;
    }
    has_matching_wildcard_edges(name, name_len, prefix, pre_len, suffix, suf_len)
}

fn has_matching_wildcard_edges(
    name: &str,
    name_len: usize,
    prefix: &str,
    pre_len: usize,
    suffix: &str,
    suf_len: usize,
) -> bool {
    name[..pre_len].eq_ignore_ascii_case(prefix)
        && name[name_len - suf_len..].eq_ignore_ascii_case(suffix)
}
