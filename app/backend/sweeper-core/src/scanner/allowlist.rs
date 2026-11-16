use super::*;

/// Bundled input for allowlisted scan runtime execution.
pub(super) struct AllowlistedScanRuntimeInput<'a, 'p> {
    pub targets: Vec<TargetDefinition>,
    pub min_age_hours: u32,
    pub redirected_temp_candidate: Option<&'a str>,
    pub elevation_target_root_overrides: &'a BTreeMap<String, String>,
    pub progress: &'p mut ScanProgressRuntime,
    pub started_unix_secs: u64,
}

pub(super) fn scan_allowlisted_targets_with_progress_runtime(
    input: AllowlistedScanRuntimeInput<'_, '_>,
) -> Result<ScanReport, ScanError> {
    let AllowlistedScanRuntimeInput {
        targets,
        min_age_hours,
        redirected_temp_candidate,
        elevation_target_root_overrides,
        progress,
        started_unix_secs,
    } = input;
    validate_target_definitions(&targets).map_err(ScanError::Targets)?;
    let ctx = AllowlistedScanCtx::new(started_unix_secs, min_age_hours, redirected_temp_candidate);
    let kf_roots = resolve_needed_known_folder_roots(&targets, elevation_target_root_overrides);
    let results = scan_allowlisted_target_list(
        targets,
        &kf_roots,
        elevation_target_root_overrides,
        &ctx,
        progress,
    )?;
    Ok(ScanReport {
        started_unix_secs,
        min_age_hours,
        targets: results,
    })
}

pub(super) struct AllowlistedScanCtx<'a> {
    now: SystemTime,
    min_age: Duration,
    redirected_temp_candidate: Option<&'a str>,
}

impl<'a> AllowlistedScanCtx<'a> {
    pub(super) fn new(
        started_unix_secs: u64,
        min_age_hours: u32,
        redirected_temp_candidate: Option<&'a str>,
    ) -> Self {
        Self {
            now: UNIX_EPOCH + Duration::from_secs(started_unix_secs),
            min_age: Duration::from_secs(u64::from(min_age_hours) * 3600),
            redirected_temp_candidate,
        }
    }
}

pub(super) fn resolve_needed_known_folder_roots(
    targets: &[TargetDefinition],
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> BTreeMap<KnownFolder, Result<CanonicalPath, String>> {
    let needed_kfs: HashSet<KnownFolder> = targets
        .iter()
        .filter(|t| !is_overridden_elevation_target(t, elevation_target_root_overrides))
        .map(|t| t.known_folder)
        .collect();
    let mut kf_roots = BTreeMap::new();
    for kf in needed_kfs {
        kf_roots.insert(kf, resolve_validated_known_folder_root(kf));
    }
    kf_roots
}

pub(super) fn is_overridden_elevation_target(
    target: &TargetDefinition,
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> bool {
    target.requires_elevation && elevation_target_root_overrides.contains_key(&target.id)
}

pub(super) fn scan_allowlisted_target_list(
    targets: Vec<TargetDefinition>,
    kf_roots: &BTreeMap<KnownFolder, Result<CanonicalPath, String>>,
    elevation_target_root_overrides: &BTreeMap<String, String>,
    ctx: &AllowlistedScanCtx<'_>,
    progress: &mut ScanProgressRuntime,
) -> Result<Vec<TargetScanResult>, ScanError> {
    let mut results = Vec::with_capacity(targets.len());
    let mut all_resolved_roots: Vec<(String, CanonicalPath)> = Vec::new();
    for target in targets {
        if progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        progress.emit(Some(&target.id), Some(&target.name));
        let out = scan_single_allowlisted_target(
            &target,
            kf_roots,
            elevation_target_root_overrides,
            &mut all_resolved_roots,
            ctx,
            progress,
        )?;
        progress.completed_targets = progress.completed_targets.saturating_add(1);
        progress.emit(None, None);
        results.push(out);
    }
    Ok(results)
}

pub(super) fn scan_single_allowlisted_target(
    target: &TargetDefinition,
    kf_roots: &BTreeMap<KnownFolder, Result<CanonicalPath, String>>,
    elevation_target_root_overrides: &BTreeMap<String, String>,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
    ctx: &AllowlistedScanCtx<'_>,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let mut out = super::empty_target_result(super::TargetResultMeta {
        target_id: target.id.clone(),
        target_name: target.name.clone(),
        tier: target.tier,
        risk: target.risk,
    });
    if let Some(done) = try_scan_elevation_override(
        target,
        elevation_target_root_overrides,
        all_resolved_roots,
        ctx,
        &mut out,
        progress,
    )? {
        return Ok(done);
    }
    let base_root = match resolve_target_base_root(target, kf_roots) {
        Ok(p) => p,
        Err(reason) => return Ok(finish_target_with_status(out, reason)),
    };
    if target.id == "redirected-temp" {
        return scan_redirected_temp_target(
            target,
            &base_root,
            all_resolved_roots,
            ctx,
            out,
            progress,
        );
    }
    scan_standard_allowlisted_target(target, &base_root, all_resolved_roots, ctx, out, progress)
}

pub(super) fn finish_target_with_status(
    mut out: TargetScanResult,
    reason: String,
) -> TargetScanResult {
    out.status_reason = Some(reason);
    out
}

pub(super) fn try_scan_elevation_override(
    target: &TargetDefinition,
    elevation_target_root_overrides: &BTreeMap<String, String>,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
    ctx: &AllowlistedScanCtx<'_>,
    out: &mut TargetScanResult,
    progress: &mut ScanProgressRuntime,
) -> Result<Option<TargetScanResult>, ScanError> {
    if !target.requires_elevation {
        return Ok(None);
    }
    let Some(override_path) = elevation_target_root_overrides.get(&target.id) else {
        return Ok(None);
    };
    let result = scan_elevation_override_path(
        target,
        override_path,
        all_resolved_roots,
        ctx,
        out,
        progress,
    )?;
    Ok(Some(result))
}

pub(super) fn scan_elevation_override_path(
    target: &TargetDefinition,
    override_path: &str,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
    ctx: &AllowlistedScanCtx<'_>,
    out: &mut TargetScanResult,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let canon = match CanonicalPath::from_existing_path(override_path) {
        Ok(v) => v.into_canonical(),
        Err(e) => {
            return Ok(finish_target_with_status(
                out.clone(),
                format!("elevation target root override unresolved: {e}"),
            ));
        }
    };
    if let Err(e) = validate_resolved_canonical_root(&canon) {
        return Ok(finish_target_with_status(
            out.clone(),
            format!("elevation target root override invalid: {e}"),
        ));
    }
    scan_validated_target_roots(
        target,
        vec![(target.id.clone(), canon)],
        all_resolved_roots,
        ctx,
        out.clone(),
        progress,
    )
}

pub(super) fn resolve_target_base_root(
    target: &TargetDefinition,
    kf_roots: &BTreeMap<KnownFolder, Result<CanonicalPath, String>>,
) -> Result<CanonicalPath, String> {
    let kf_res = kf_roots
        .get(&target.known_folder)
        .cloned()
        .unwrap_or_else(|| Err("known folder not resolved".to_string()));
    kf_res.map_err(|e| format!("known folder resolution failed: {e}"))
}

pub(super) fn scan_redirected_temp_target(
    target: &TargetDefinition,
    base_root: &CanonicalPath,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
    ctx: &AllowlistedScanCtx<'_>,
    mut out: TargetScanResult,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let temp_guard = match resolve_redirected_temp_guard(base_root, ctx.redirected_temp_candidate) {
        Ok(g) => g,
        Err(reason) => return Ok(finish_target_with_status(out, reason)),
    };
    match temp_guard {
        TempGuard::Use(path) => {
            let roots = match validate_and_collect_roots_for_target(target, base_root, Some(&path))
            {
                Ok(r) => r,
                Err(e) => return Ok(finish_target_with_status(out, e)),
            };
            scan_validated_target_roots(target, roots, all_resolved_roots, ctx, out, progress)
        }
        TempGuard::Disabled(reason) => {
            out.status_reason = Some(format!("redirected-temp disabled: {reason}"));
            Ok(out)
        }
    }
}

pub(super) fn resolve_redirected_temp_guard(
    base_root: &CanonicalPath,
    redirected_temp_candidate: Option<&str>,
) -> Result<TempGuard, String> {
    let Some(candidate) = redirected_temp_candidate else {
        return Ok(evaluate_redirected_temp(base_root));
    };
    let user_temp = join_and_canonicalize(base_root, "Temp")
        .map_err(|e| format!("redirected-temp disabled: failed to resolve user-temp: {e}"))?;
    Ok(evaluate_redirected_temp_candidate(
        Some(candidate),
        &user_temp,
    ))
}

pub(super) fn scan_standard_allowlisted_target(
    target: &TargetDefinition,
    base_root: &CanonicalPath,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
    ctx: &AllowlistedScanCtx<'_>,
    mut out: TargetScanResult,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let roots = match validate_and_collect_roots_for_target(target, base_root, None) {
        Ok(r) => r,
        Err(e) => return Ok(finish_target_with_status(out, e)),
    };
    if roots.is_empty() {
        out.status_reason = Some("not present".to_string());
        return Ok(out);
    }
    scan_validated_target_roots(target, roots, all_resolved_roots, ctx, out, progress)
}

pub(super) fn scan_validated_target_roots(
    target: &TargetDefinition,
    roots: Vec<(String, CanonicalPath)>,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
    ctx: &AllowlistedScanCtx<'_>,
    mut out: TargetScanResult,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    if let Err(reason) = validate_target_roots_policy(&roots, all_resolved_roots) {
        out.status_reason = Some(reason);
        return Ok(out);
    }
    for (rid, r) in &roots {
        all_resolved_roots.push((rid.clone(), r.clone()));
    }
    out.resolved_paths = roots.iter().map(|(_, p)| p.to_string()).collect();
    scan_resolved_roots(target, &roots, ctx.min_age, ctx.now, &mut out, progress)?;
    Ok(out)
}

pub(super) fn scan_resolved_roots(
    target: &TargetDefinition,
    roots: &[(String, CanonicalPath)],
    min_age: Duration,
    now: SystemTime,
    out: &mut TargetScanResult,
    progress: &mut ScanProgressRuntime,
) -> Result<(), ScanError> {
    out.scanned = true;
    let mut next_item_seq: u64 = 0;
    for (_root_id, root) in roots {
        if progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        walk::scan_one_root_iterative(
            target,
            root,
            min_age,
            now,
            out,
            &mut next_item_seq,
            progress,
        )?;
    }
    Ok(())
}

pub(super) fn validate_and_collect_roots_for_target(
    target: &TargetDefinition,
    known_folder_root: &CanonicalPath,
    override_root: Option<&CanonicalPath>,
) -> Result<Vec<(String, CanonicalPath)>, String> {
    if target.id == "redirected-temp" {
        if let Some(p) = override_root {
            let root = validate_single_resolved_root(target, p.clone())?;
            return Ok(vec![("redirected-temp".to_string(), root)]);
        }
    }

    let rel = &target.relative_subpath;
    let segments: Vec<&str> = rel.split('\\').collect();
    let mut candidates = vec![PathBuf::from(known_folder_root.as_str())];
    for seg in segments {
        if seg.contains('*') {
            candidates = expand_wildcard_segment(candidates, seg);
        } else {
            for c in &mut candidates {
                c.push(seg);
            }
        }
    }

    let mut roots = Vec::new();
    for c in candidates {
        if !c.exists() {
            continue;
        }
        match validate_resolved_root_path(&c) {
            Ok(p) => roots.push((target.id.clone(), p)),
            Err(e) => return Err(format!("resolved target root invalid: {e}")),
        }
    }
    roots.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    roots.dedup_by(|a, b| a.1.is_same_path(&b.1));
    Ok(roots)
}

pub(super) fn expand_wildcard_segment(candidates: Vec<PathBuf>, seg: &str) -> Vec<PathBuf> {
    let mut next = Vec::new();
    for cand in candidates {
        if !cand.exists() {
            continue;
        }
        let entries = match sweeper_winfs::read_dir_entries(&cand) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for ent in entries {
            if is_eligible_wildcard_entry(&ent, seg) {
                next.push(ent.path);
            }
        }
    }
    next
}

pub(super) fn is_eligible_wildcard_entry(ent: &sweeper_winfs::DirEntry, seg: &str) -> bool {
    if !ent.is_dir {
        return false;
    }
    let name = ent.name.to_string_lossy();
    if !segment_pattern_matches(&name, seg) {
        return false;
    }
    // Intermediate wildcard expansion: only non-reparse directories
    if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
        return false;
    }
    // Defense-in-depth: ensure not a link point via handle.
    !sweeper_winfs::open_for_inspection(&ent.path)
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(true)
}

pub(super) fn validate_single_resolved_root(
    _target: &TargetDefinition,
    path: CanonicalPath,
) -> Result<CanonicalPath, String> {
    validate_resolved_canonical_root(&path).map(|_| path)
}

pub(super) fn validate_resolved_root_path(path: &Path) -> Result<CanonicalPath, String> {
    let validated =
        CanonicalPath::from_existing_path(&path.to_string_lossy()).map_err(|e| e.to_string())?;
    let canon = validated.into_canonical();
    validate_resolved_canonical_root(&canon)?;
    Ok(canon)
}

pub(super) fn validate_resolved_canonical_root(root: &CanonicalPath) -> Result<(), String> {
    if root.segment_count() < 3 {
        return Err("resolved target root must have depth >= 3 segments".to_string());
    }
    if policy::is_subtree_protected(root) {
        return Err("resolved target root is subtree-protected".to_string());
    }
    if policy::is_ancestor_of_subtree_protected(root) {
        return Err("resolved target root is ancestor of a subtree-protected path".to_string());
    }
    if policy::is_protected_container_root(root) {
        return Err("resolved target root is a protected container root".to_string());
    }
    if sweeper_winfs::open_for_inspection(Path::new(root.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(false)
    {
        return Err("resolved target root is a link point".to_string());
    }
    // Reject roots that traverse a link point ancestor.
    if path_has_link_point_ancestor(root)? {
        return Err("resolved target root traverses a link point ancestor".to_string());
    }
    Ok(())
}

pub(super) fn validate_target_roots_policy(
    roots: &[(String, CanonicalPath)],
    all_existing: &[(String, CanonicalPath)],
) -> Result<(), String> {
    for (_, r) in roots {
        // Also ensure not equal/under subtree-protected in broader sense? is_subtree_protected covers exact/descendants.
        if policy::is_subtree_protected(r) || policy::is_ancestor_of_subtree_protected(r) {
            return Err(format!(
                "target root not allowed by protected path policy: {r}"
            ));
        }
        if policy::is_protected_container_root(r) {
            return Err(format!("target root is protected container root: {r}"));
        }
    }
    // Zero unintended overlap between resolved targets (same or ancestor/descendant).
    for (_, a) in roots {
        for (_, b) in all_existing {
            if paths_overlap(a, b) {
                return Err(format!(
                    "overlap between resolved targets detected: {} and {}",
                    a, b
                ));
            }
        }
    }
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            if paths_overlap(&roots[i].1, &roots[j].1) {
                return Err(format!(
                    "overlap between resolved targets detected: {} and {}",
                    roots[i].1, roots[j].1
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn paths_overlap(a: &CanonicalPath, b: &CanonicalPath) -> bool {
    a.is_same_path(b) || a.is_self_or_ancestor_of(b) || b.is_self_or_ancestor_of(a)
}

pub(super) fn segment_pattern_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return name.eq_ignore_ascii_case(pattern);
    }
    // Allowlist lint enforces a single '*' in intermediate segment patterns.
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return false;
    }
    let (prefix, suffix) = (parts[0], parts[1]);
    let name_len = name.len();
    let pre_len = prefix.len();
    let suf_len = suffix.len();
    if name_len < pre_len + suf_len {
        return false;
    }
    name[..pre_len].eq_ignore_ascii_case(prefix)
        && name[name_len - suf_len..].eq_ignore_ascii_case(suffix)
}

pub(super) fn resolve_validated_known_folder_root(
    kf: KnownFolder,
) -> Result<CanonicalPath, String> {
    let id = map_known_folder(kf);
    let path = sweeper_winfs::resolve_known_folder(id).map_err(|e| e.to_string())?;
    let canon = CanonicalPath::from_existing_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())?
        .into_canonical();
    // Known folder roots themselves can be redirected; enforce depth/protected checks for key roots.
    match kf {
        KnownFolder::UserProfile | KnownFolder::LocalAppData | KnownFolder::RoamingAppData => {
            if canon.segment_count() < 3 {
                return Err("known folder root depth < 3".to_string());
            }
            if policy::is_subtree_protected(&canon)
                || policy::is_ancestor_of_subtree_protected(&canon)
                || policy::is_protected_container_root(&canon)
            {
                return Err("known folder root violates protected path policy".to_string());
            }
            if path_has_link_point_ancestor(&canon)? {
                return Err("known folder root traverses a link point".to_string());
            }
        }
        KnownFolder::Windows | KnownFolder::ProgramData => {
            // These are container roots in general; allowed as anchors for allowlisted subfolders only.
            // We still require canonical resolution succeeded.
        }
    }
    Ok(canon)
}

pub(super) fn path_has_link_point_ancestor(path: &CanonicalPath) -> Result<bool, String> {
    // Check each existing prefix path component from root downward (excluding drive root string form edge cases).
    let drive = path.drive_prefix();
    let mut cur = PathBuf::from(format!(r"{}\", drive));
    for seg in path.segments() {
        cur.push(seg);
        match sweeper_winfs::open_for_inspection(&cur)
            .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        {
            Ok(ReparseTagClass::LinkPoint) => return Ok(true),
            Ok(_) => {}
            Err(WinfsError::Win32 { code: 2 | 3, .. }) => break, // not found (shouldn't happen for validated root)
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}

pub(super) fn evaluate_redirected_temp(local_appdata_root: &CanonicalPath) -> TempGuard {
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
    let Some(temp_env) = temp_candidate.filter(|v| !v.trim().is_empty()) else {
        return TempGuard::Disabled("%TEMP% not set".to_string());
    };
    if let Ok(root) = crate::paths::parse_drive_root(temp_env.trim()) {
        // Drive root (e.g. C:\) is always depth < 3 and protected; auto-disable deterministically.
        let _ = root;
        return TempGuard::Disabled("depth < 3 (drive root)".to_string());
    }
    let temp_canon = match CanonicalPath::from_existing_path(temp_env) {
        Ok(v) => v.into_canonical(),
        Err(e) => return TempGuard::Disabled(format!("%TEMP% unresolved: {e}")),
    };
    if temp_canon.is_same_path(user_temp_canonical) {
        return TempGuard::Disabled("identical to user-temp".to_string());
    }
    if temp_canon.segment_count() < 3 {
        return TempGuard::Disabled("depth < 3".to_string());
    }
    if policy::is_subtree_protected(&temp_canon)
        || policy::is_ancestor_of_subtree_protected(&temp_canon)
        || policy::is_protected_container_root(&temp_canon)
    {
        return TempGuard::Disabled("resolves to a protected path".to_string());
    }
    match path_has_link_point_ancestor(&temp_canon) {
        Ok(true) => return TempGuard::Disabled("traverses a link point".to_string()),
        Ok(false) => {}
        Err(e) => return TempGuard::Disabled(format!("link-point inspection failed: {e}")),
    }
    // Ensure not a link point itself.
    if sweeper_winfs::open_for_inspection(Path::new(temp_canon.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(false)
    {
        return TempGuard::Disabled("%TEMP% path is a link point".to_string());
    }
    TempGuard::Use(temp_canon)
}

pub(super) fn join_and_canonicalize(
    base: &CanonicalPath,
    child: &str,
) -> Result<CanonicalPath, String> {
    let p = Path::new(base.as_str()).join(child);
    CanonicalPath::from_existing_path(&p.to_string_lossy())
        .map(|v| v.into_canonical())
        .map_err(|e| e.to_string())
}

pub(super) fn map_known_folder(kf: KnownFolder) -> KnownFolderId {
    match kf {
        KnownFolder::LocalAppData => KnownFolderId::LocalAppData,
        KnownFolder::RoamingAppData => KnownFolderId::RoamingAppData,
        KnownFolder::UserProfile => KnownFolderId::UserProfile,
        KnownFolder::Windows => KnownFolderId::Windows,
        KnownFolder::ProgramData => KnownFolderId::ProgramData,
    }
}

pub(super) fn filetime_to_unix_secs(ft: (u32, u32)) -> u64 {
    let v = ((ft.0 as u64) << 32) | (ft.1 as u64);
    // FILETIME is 100-ns intervals since 1601-01-01 UTC.
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    if v <= EPOCH_DIFF_100NS {
        0
    } else {
        (v - EPOCH_DIFF_100NS) / 10_000_000
    }
}

pub(super) fn system_time_to_unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
