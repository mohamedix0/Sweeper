use super::*;

use crate::paths::CanonicalPath;

/// Verify Gate E read-only invariants against a scan report produced from embedded Tier 1/2 targets.
///
/// Returns a list of violation messages (empty means pass).
pub fn gate_e_violations(report: &ScanReport) -> Vec<String> {
    let mut violations = Vec::new();
    let mut seen_roots: Vec<(String, CanonicalPath)> = Vec::new();
    for t in &report.targets {
        collect_target_gate_e_violations(t, &mut seen_roots, &mut violations);
    }
    violations
}

fn collect_target_gate_e_violations(
    target: &TargetScanResult,
    seen_roots: &mut Vec<(String, CanonicalPath)>,
    violations: &mut Vec<String>,
) {
    if !target.scanned {
        return;
    }
    for p in &target.resolved_paths {
        collect_path_gate_e_violations(target, p, seen_roots, violations);
    }
}

fn collect_path_gate_e_violations(
    target: &TargetScanResult,
    path: &str,
    seen_roots: &mut Vec<(String, CanonicalPath)>,
    violations: &mut Vec<String>,
) {
    let Some(canon) = resolve_gate_e_path(&target.target_id, path, violations) else {
        return;
    };
    check_gate_e_path_policy(target, &canon, violations);
    check_gate_e_path_reparse(target, &canon, violations);
    check_gate_e_path_overlap(target, &canon, seen_roots, violations);
    seen_roots.push((target.target_id.clone(), canon));
}

fn resolve_gate_e_path(
    target_id: &str,
    path: &str,
    violations: &mut Vec<String>,
) -> Option<CanonicalPath> {
    match CanonicalPath::from_existing_path(path) {
        Ok(v) => Some(v.into_canonical()),
        Err(e) => {
            violations.push(format!(
                "target {target_id} resolved path not resolvable: {path} ({e})"
            ));
            None
        }
    }
}

fn check_gate_e_path_policy(
    target: &TargetScanResult,
    canon: &CanonicalPath,
    violations: &mut Vec<String>,
) {
    if is_subtree_protected_violation(canon) {
        violations.push(format!(
            "target {} overlaps subtree-protected path: {}",
            target.target_id, canon
        ));
    }
    if policy::is_protected_container_root(canon) {
        violations.push(format!(
            "target {} targets protected container root: {}",
            target.target_id, canon
        ));
    }
}

fn is_subtree_protected_violation(canon: &CanonicalPath) -> bool {
    policy::is_subtree_protected(canon) || policy::is_ancestor_of_subtree_protected(canon)
}

fn check_gate_e_path_reparse(
    target: &TargetScanResult,
    canon: &CanonicalPath,
    violations: &mut Vec<String>,
) {
    let inspect = sweeper_winfs::open_for_inspection(std::path::Path::new(canon.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h));
    match inspect {
        Ok(ReparseTagClass::LinkPoint) => violations.push(format!(
            "target {} resolved root is link point: {}",
            target.target_id, canon
        )),
        Ok(_) => {}
        Err(e) => violations.push(format!(
            "target {} failed reparse inspection for {}: {e}",
            target.target_id, canon
        )),
    }
}

fn check_gate_e_path_overlap(
    target: &TargetScanResult,
    canon: &CanonicalPath,
    seen_roots: &[(String, CanonicalPath)],
    violations: &mut Vec<String>,
) {
    for (other_id, other) in seen_roots {
        if allowlist::paths_overlap(canon, other) {
            violations.push(format!(
                "overlap between resolved targets: {} ({}) and {} ({})",
                target.target_id, canon, other_id, other
            ));
        }
    }
}
