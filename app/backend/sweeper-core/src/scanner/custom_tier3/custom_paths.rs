use super::CustomScanCtx;
use super::*;

pub(super) fn append_custom_path_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    ctx: &mut CustomScanCtx<'_>,
) -> Result<(), ScanError> {
    for (idx, raw) in settings.custom_paths.iter().enumerate() {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        report.targets.push(scan_custom_path_target(idx, raw, ctx)?);
    }
    Ok(())
}

pub(super) fn scan_custom_path_target(
    idx: usize,
    raw_path: &str,
    ctx: &mut CustomScanCtx<'_>,
) -> Result<TargetScanResult, ScanError> {
    let mut out = empty_custom_path_result(idx);
    emit_target_start(ctx, &out)?;
    if let Err((reason, diag)) = validate_custom_path_for_scan(raw_path) {
        return Ok(finalize_custom_target(out, ctx, reason, diag));
    }
    let canon = match resolve_custom_path_for_scan(raw_path) {
        Ok(c) => c,
        Err((reason, diag)) => return Ok(finalize_custom_target(out, ctx, reason, diag)),
    };
    scan_custom_canonical_target(&mut out, canon, ctx)?;
    complete_target(ctx);
    Ok(out)
}

fn emit_target_start(ctx: &mut CustomScanCtx<'_>, out: &TargetScanResult) -> Result<(), ScanError> {
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    ctx.progress
        .emit(Some(&out.target_id), Some(&out.target_name));
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    Ok(())
}

fn empty_custom_path_result(idx: usize) -> TargetScanResult {
    super::super::empty_target_result(super::super::TargetResultMeta {
        target_id: format!("custom-path-{}", idx + 1),
        target_name: "Custom Path".to_string(),
        tier: Tier::Tier2,
        risk: Risk::Caution,
    })
}

fn validate_custom_path_for_scan(raw_path: &str) -> Result<(), (String, String)> {
    let verdict = policy::validate_custom_path(raw_path);
    if verdict.allowed {
        return Ok(());
    }
    Err(custom_path_invalid_reason(raw_path, &verdict.reason))
}

fn custom_path_invalid_reason(raw_path: &str, reason: &str) -> (String, String) {
    (
        format!("custom path invalid/missing (no fallback): {reason}"),
        format!("custom path rejected: {raw_path} ({reason})"),
    )
}

fn resolve_custom_path_for_scan(raw_path: &str) -> Result<CanonicalPath, (String, String)> {
    let canon = CanonicalPath::from_existing_path(raw_path)
        .map(|v| v.into_canonical())
        .map_err(|e| custom_path_unresolved_reason(raw_path, &e.to_string()))?;
    allowlist::validate_resolved_canonical_root(&canon)
        .map_err(|e| custom_path_scan_time_reason(&e))?;
    Ok(canon)
}

fn custom_path_unresolved_reason(raw_path: &str, err: &str) -> (String, String) {
    (
        format!("custom path missing/unresolvable (no fallback): {err}"),
        format!("custom path unresolved: {raw_path} ({err})"),
    )
}

fn custom_path_scan_time_reason(err: &str) -> (String, String) {
    (
        format!("custom path not allowed at scan time: {err}"),
        format!("custom path rejected at scan time: {err}"),
    )
}

fn finalize_custom_target(
    mut out: TargetScanResult,
    ctx: &mut CustomScanCtx<'_>,
    status_reason: String,
    diagnostic: String,
) -> TargetScanResult {
    out.scanned = false;
    out.status_reason = Some(status_reason);
    out.skipped_diagnostics.push(diagnostic);
    complete_target(ctx);
    out
}

fn scan_custom_canonical_target(
    out: &mut TargetScanResult,
    canon: CanonicalPath,
    ctx: &mut CustomScanCtx<'_>,
) -> Result<(), ScanError> {
    let td = custom_path_target_definition(&out.target_id, &out.target_name, ctx.min_age);
    out.resolved_paths = vec![canon.to_string()];
    let roots = [(out.target_id.clone(), canon)];
    let mut scan_ctx = allowlist::ResolvedRootsScanCtx {
        target: &td,
        roots: &roots,
        min_age: ctx.min_age,
        now: ctx.now,
        progress: ctx.progress,
    };
    allowlist::scan_resolved_roots(&mut scan_ctx, out)
}

fn complete_target(ctx: &mut CustomScanCtx<'_>) {
    ctx.progress.completed_targets = ctx.progress.completed_targets.saturating_add(1);
    ctx.progress.emit(None, None);
}

fn custom_path_target_definition(
    target_id: &str,
    target_name: &str,
    min_age: Duration,
) -> TargetDefinition {
    TargetDefinition {
        id: target_id.to_string(),
        name: target_name.to_string(),
        tier: Tier::Tier2,
        risk: Risk::Caution,
        known_folder: KnownFolder::LocalAppData,
        relative_subpath: String::new(),
        scope: sweeper_knowledge::Scope::Contents,
        min_age_hours: (min_age.as_secs() / 3600) as u32,
        description: "User-validated custom path (contents only).".to_string(),
        requires_elevation: false,
    }
}
