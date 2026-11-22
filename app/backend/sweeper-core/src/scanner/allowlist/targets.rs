use super::*;

pub(super) struct TargetScanRun<'env, 'roots, 'ctx, 'p> {
    pub(super) env: &'env mut AllowlistedScanEnv<'ctx, 'p>,
    pub(super) all_resolved_roots: &'roots mut Vec<(String, CanonicalPath)>,
}

pub(in crate::scanner) struct ResolvedRootsScanCtx<'a, 'p> {
    pub(in crate::scanner) target: &'a TargetDefinition,
    pub(in crate::scanner) roots: &'a [(String, CanonicalPath)],
    pub(in crate::scanner) min_age: std::time::Duration,
    pub(in crate::scanner) now: std::time::SystemTime,
    pub(in crate::scanner) progress: &'p mut ScanProgressRuntime,
}

struct TargetRoots {
    _target_id: String,
    roots: Vec<(String, CanonicalPath)>,
}

pub(in super::super) fn scan_allowlisted_target_list(
    targets: Vec<TargetDefinition>,
    env: &mut AllowlistedScanEnv<'_, '_>,
) -> Result<Vec<TargetScanResult>, ScanError> {
    let mut results = Vec::with_capacity(targets.len());
    let mut all_resolved_roots: Vec<(String, CanonicalPath)> = Vec::new();
    let mut run = TargetScanRun {
        env,
        all_resolved_roots: &mut all_resolved_roots,
    };
    for target in targets {
        if run.env.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        run.env.progress.emit(Some(&target.id), Some(&target.name));
        let out = scan_single_allowlisted_target(&target, &mut run)?;
        run.env.progress.completed_targets = run.env.progress.completed_targets.saturating_add(1);
        run.env.progress.emit(None, None);
        results.push(out);
    }
    Ok(results)
}

fn scan_single_allowlisted_target(
    target: &TargetDefinition,
    run: &mut TargetScanRun<'_, '_, '_, '_>,
) -> Result<TargetScanResult, ScanError> {
    let mut out = super::super::empty_target_result(super::super::TargetResultMeta {
        target_id: target.id.clone(),
        target_name: target.name.clone(),
        tier: target.tier,
        risk: target.risk,
    });
    if let Some(done) = try_scan_elevation_override(target, run, &mut out)? {
        return Ok(done);
    }
    let base_root = match resolve_target_base_root(target, run.env.kf_roots) {
        Ok(p) => p,
        Err(reason) => return Ok(finish_target_with_status(out, reason)),
    };
    if target.id == "redirected-temp" {
        return scan_redirected_temp_target(target, &base_root, run, out);
    }
    scan_standard_allowlisted_target(target, &base_root, run, out)
}

fn finish_target_with_status(mut out: TargetScanResult, reason: String) -> TargetScanResult {
    out.status_reason = Some(reason);
    out
}

fn try_scan_elevation_override(
    target: &TargetDefinition,
    run: &mut TargetScanRun<'_, '_, '_, '_>,
    out: &mut TargetScanResult,
) -> Result<Option<TargetScanResult>, ScanError> {
    if !roots::is_overridden_elevation_target(target, run.env.elevation_target_root_overrides) {
        return Ok(None);
    }
    let override_path = run.env.elevation_target_root_overrides[&target.id].clone();
    let result = scan_elevation_override_path(target, &override_path, run, out)?;
    Ok(Some(result))
}

fn scan_elevation_override_path(
    target: &TargetDefinition,
    override_path: &str,
    run: &mut TargetScanRun<'_, '_, '_, '_>,
    out: &mut TargetScanResult,
) -> Result<TargetScanResult, ScanError> {
    let canon = match canonicalize_elevation_override(override_path) {
        Ok(c) => c,
        Err(reason) => return Ok(finish_target_with_status(out.clone(), reason)),
    };
    let roots = vec![(target.id.clone(), canon)];
    scan_validated_target_roots(target, roots, run, out.clone())
}

fn canonicalize_elevation_override(override_path: &str) -> Result<CanonicalPath, String> {
    let canon = CanonicalPath::from_existing_path(override_path)
        .map(|v| v.into_canonical())
        .map_err(|e| format!("elevation target root override unresolved: {e}"))?;
    super::policy::validate_resolved_canonical_root(&canon)
        .map_err(|e| format!("elevation target root override invalid: {e}"))?;
    Ok(canon)
}

fn resolve_target_base_root(
    target: &TargetDefinition,
    kf_roots: &BTreeMap<KnownFolder, Result<CanonicalPath, String>>,
) -> Result<CanonicalPath, String> {
    let kf_res = kf_roots
        .get(&target.known_folder)
        .cloned()
        .unwrap_or_else(|| Err("known folder not resolved".to_string()));
    kf_res.map_err(|e| format!("known folder resolution failed: {e}"))
}

fn scan_redirected_temp_target(
    target: &TargetDefinition,
    base_root: &CanonicalPath,
    run: &mut TargetScanRun<'_, '_, '_, '_>,
    mut out: TargetScanResult,
) -> Result<TargetScanResult, ScanError> {
    let fallback_guard = temp_guard::evaluate_redirected_temp(base_root);
    let temp_guard = match resolve_redirected_temp_guard(
        base_root,
        run.env.ctx.redirected_temp_candidate,
        fallback_guard,
    ) {
        Ok(g) => g,
        Err(reason) => return Ok(finish_target_with_status(out, reason)),
    };
    match temp_guard {
        TempGuard::Use(path) => {
            let roots = match collect_target_roots(target, base_root, Some(&path)) {
                Ok(r) => r,
                Err(e) => return Ok(finish_target_with_status(out, e)),
            };
            scan_validated_target_roots(target, roots, run, out)
        }
        TempGuard::Disabled(reason) => {
            out.status_reason = Some(format!("redirected-temp disabled: {reason}"));
            Ok(out)
        }
    }
}

fn resolve_redirected_temp_guard(
    base_root: &CanonicalPath,
    candidate: Option<&str>,
    fallback: TempGuard,
) -> Result<TempGuard, String> {
    if candidate.is_none() {
        return Ok(fallback);
    }
    let user_temp = temp_guard::join_and_canonicalize(base_root, "Temp")
        .map_err(|e| format!("redirected-temp disabled: failed to resolve user-temp: {e}"))?;
    Ok(temp_guard::evaluate_redirected_temp_candidate(
        candidate, &user_temp,
    ))
}

fn scan_standard_allowlisted_target(
    target: &TargetDefinition,
    base_root: &CanonicalPath,
    run: &mut TargetScanRun<'_, '_, '_, '_>,
    mut out: TargetScanResult,
) -> Result<TargetScanResult, ScanError> {
    let roots = match collect_target_roots(target, base_root, None) {
        Ok(r) => r,
        Err(e) => return Ok(finish_target_with_status(out, e)),
    };
    if roots.is_empty() {
        out.status_reason = Some("not present".to_string());
        return Ok(out);
    }
    scan_validated_target_roots(target, roots, run, out)
}

fn collect_target_roots(
    target: &TargetDefinition,
    known_folder_root: &CanonicalPath,
    override_root: Option<&CanonicalPath>,
) -> Result<Vec<(String, CanonicalPath)>, String> {
    super::wildcard::validate_and_collect_roots_for_target(target, known_folder_root, override_root)
}

fn scan_validated_target_roots(
    target: &TargetDefinition,
    roots: Vec<(String, CanonicalPath)>,
    run: &mut TargetScanRun<'_, '_, '_, '_>,
    mut out: TargetScanResult,
) -> Result<TargetScanResult, ScanError> {
    let grouped = TargetRoots {
        _target_id: target.id.clone(),
        roots,
    };
    if let Err(reason) =
        super::policy::validate_target_roots_policy(&grouped.roots, run.all_resolved_roots)
    {
        out.status_reason = Some(reason);
        return Ok(out);
    }
    register_target_roots(&grouped, run.all_resolved_roots);
    out.resolved_paths = grouped.roots.iter().map(|(_, p)| p.to_string()).collect();
    let mut ctx = ResolvedRootsScanCtx {
        target,
        roots: &grouped.roots,
        min_age: run.env.ctx.min_age,
        now: run.env.ctx.now,
        progress: run.env.progress,
    };
    scan_resolved_roots(&mut ctx, &mut out)?;
    Ok(out)
}

fn register_target_roots(
    grouped: &TargetRoots,
    all_resolved_roots: &mut Vec<(String, CanonicalPath)>,
) {
    for (rid, r) in &grouped.roots {
        all_resolved_roots.push((rid.clone(), r.clone()));
    }
}

pub(in crate::scanner) fn scan_resolved_roots(
    ctx: &mut ResolvedRootsScanCtx<'_, '_>,
    out: &mut TargetScanResult,
) -> Result<(), ScanError> {
    out.scanned = true;
    let mut next_item_seq: u64 = 0;
    for (_root_id, root) in ctx.roots {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        let mut req = walk::RootWalk {
            target: ctx.target,
            root,
            min_age: ctx.min_age,
            now: ctx.now,
            out,
            next_item_seq: &mut next_item_seq,
            progress: ctx.progress,
            include_all_entries: false,
        };
        if ctx.target.scope == sweeper_knowledge::Scope::WholeCache {
            walk::scan_one_root_whole_cache(&mut req)?;
        } else {
            walk::scan_one_root_iterative(&mut req)?;
        }
    }
    Ok(())
}
