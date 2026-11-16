use super::*;

pub(in crate::scanner) fn filetime_to_unix_secs(ft: (u32, u32)) -> u64 {
    let v = ((ft.0 as u64) << 32) | (ft.1 as u64);
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    if v <= EPOCH_DIFF_100NS {
        0
    } else {
        (v - EPOCH_DIFF_100NS) / 10_000_000
    }
}

pub(in super::super) fn evaluate_redirected_temp(local_appdata_root: &CanonicalPath) -> TempGuard {
    let user_temp = match join_and_canonicalize(local_appdata_root, "Temp") {
        Ok(p) => p,
        Err(e) => return TempGuard::Disabled(format!("failed to resolve user-temp: {e}")),
    };
    let temp_env = sweeper_winfs::env_var("TEMP").filter(|v| !v.trim().is_empty());
    evaluate_redirected_temp_candidate(temp_env.as_deref(), &user_temp)
}

/// Evaluate redirected `%TEMP%` guard for an injected candidate path and canonical user-temp root.
///
/// This is a deterministic, testable seam (no global env mutation). `temp_candidate` should be the
/// `%TEMP%` path value to validate, or `None` when unset/empty.
pub fn evaluate_redirected_temp_candidate(
    temp_candidate: Option<&str>,
    user_temp_canonical: &CanonicalPath,
) -> TempGuard {
    let Some(temp_env) = non_empty_temp_candidate(temp_candidate) else {
        return TempGuard::Disabled("%TEMP% not set".to_string());
    };
    if is_drive_root_candidate(temp_env) {
        return TempGuard::Disabled("depth < 3 (drive root)".to_string());
    }
    let temp_canon = match canonicalize_temp_candidate(temp_env) {
        Ok(v) => v,
        Err(reason) => return TempGuard::Disabled(reason),
    };
    validate_redirected_temp_canon(&temp_canon, user_temp_canonical)
}

fn non_empty_temp_candidate(temp_candidate: Option<&str>) -> Option<&str> {
    temp_candidate.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn is_drive_root_candidate(temp_env: &str) -> bool {
    crate::paths::parse_drive_root(temp_env).is_ok()
}

fn canonicalize_temp_candidate(temp_env: &str) -> Result<CanonicalPath, String> {
    CanonicalPath::from_existing_path(temp_env)
        .map(|v| v.into_canonical())
        .map_err(|e| format!("%TEMP% unresolved: {e}"))
}

fn validate_redirected_temp_canon(
    temp_canon: &CanonicalPath,
    user_temp_canonical: &CanonicalPath,
) -> TempGuard {
    if let Some(reason) = redirected_temp_rejection_reason(temp_canon, user_temp_canonical) {
        return TempGuard::Disabled(reason);
    }
    if let Some(reason) = redirected_temp_link_reason(temp_canon) {
        return TempGuard::Disabled(reason);
    }
    TempGuard::Use(temp_canon.clone())
}

fn redirected_temp_rejection_reason(
    temp_canon: &CanonicalPath,
    user_temp_canonical: &CanonicalPath,
) -> Option<String> {
    if temp_canon.is_same_path(user_temp_canonical) {
        return Some("identical to user-temp".to_string());
    }
    if temp_canon.segment_count() < 3 {
        return Some("depth < 3".to_string());
    }
    if is_protected_temp_path(temp_canon) {
        return Some("resolves to a protected path".to_string());
    }
    None
}

fn is_protected_temp_path(temp_canon: &CanonicalPath) -> bool {
    if crate::policy::is_subtree_protected(temp_canon) {
        return true;
    }
    if crate::policy::is_ancestor_of_subtree_protected(temp_canon) {
        return true;
    }
    crate::policy::is_protected_container_root(temp_canon)
}

fn redirected_temp_link_reason(temp_canon: &CanonicalPath) -> Option<String> {
    match roots::path_has_link_point_ancestor(temp_canon) {
        Ok(true) => return Some("traverses a link point".to_string()),
        Ok(false) => {}
        Err(e) => return Some(format!("link-point inspection failed: {e}")),
    }
    if is_link_point_self(temp_canon) {
        return Some("%TEMP% path is a link point".to_string());
    }
    None
}

fn is_link_point_self(path: &CanonicalPath) -> bool {
    sweeper_winfs::open_for_inspection(Path::new(path.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(false)
}

pub(in super::super) fn join_and_canonicalize(
    base: &CanonicalPath,
    child: &str,
) -> Result<CanonicalPath, String> {
    let p = Path::new(base.as_str()).join(child);
    CanonicalPath::from_existing_path(&p.to_string_lossy())
        .map(|v| v.into_canonical())
        .map_err(|e| e.to_string())
}
