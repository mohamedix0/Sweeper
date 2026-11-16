use super::Tier3RunCtx;
use super::*;

#[derive(Clone, Copy)]
struct CustomPathSpec<'a> {
    idx: usize,
    raw: &'a str,
}

struct ScanRejection {
    status_reason: String,
    diagnostic: String,
}

pub(super) fn append_custom_path_targets(
    report: &mut ScanReport,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<(), ScanError> {
    for (idx, raw) in ctx.settings.custom_paths.iter().enumerate() {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        let spec = CustomPathSpec { idx, raw };
        report.targets.push(scan_custom_path_target(spec, ctx)?);
    }
    Ok(())
}

fn scan_custom_path_target(
    spec: CustomPathSpec<'_>,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<TargetScanResult, ScanError> {
    let mut out = empty_custom_path_result(spec.idx);
    emit_target_start(ctx, &out)?;
    if let Err(rejection) = validate_custom_path_for_scan(spec) {
        return Ok(finalize_custom_target(out, ctx, rejection));
    }
    let canon = match resolve_custom_path_for_scan(spec) {
        Ok(c) => c,
        Err(rejection) => return Ok(finalize_custom_target(out, ctx, rejection)),
    };
    scan_custom_canonical_target(&mut out, canon, ctx)?;
    complete_target(ctx);
    Ok(out)
}

fn emit_target_start(
    ctx: &mut Tier3RunCtx<'_, '_>,
    out: &TargetScanResult,
) -> Result<(), ScanError> {
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

fn validate_custom_path_for_scan(spec: CustomPathSpec<'_>) -> Result<(), ScanRejection> {
    let verdict = policy::validate_custom_path(spec.raw);
    if verdict.allowed {
        return Ok(());
    }
    Err(custom_path_invalid_reason(spec, &verdict.reason))
}

fn custom_path_invalid_reason(spec: CustomPathSpec<'_>, reason: &str) -> ScanRejection {
    ScanRejection {
        status_reason: format!("custom path invalid/missing (no fallback): {reason}"),
        diagnostic: format!("custom path rejected: {} ({reason})", spec.raw),
    }
}

fn resolve_custom_path_for_scan(spec: CustomPathSpec<'_>) -> Result<CanonicalPath, ScanRejection> {
    let canon = CanonicalPath::from_existing_path(spec.raw)
        .map(|v| v.into_canonical())
        .map_err(|e| custom_path_unresolved_reason(spec, &e.to_string()))?;
    allowlist::validate_resolved_canonical_root(&canon)
        .map_err(|e| custom_path_scan_time_reason(&e))?;
    Ok(canon)
}

fn custom_path_unresolved_reason(spec: CustomPathSpec<'_>, err: &str) -> ScanRejection {
    ScanRejection {
        status_reason: format!("custom path missing/unresolvable (no fallback): {err}"),
        diagnostic: format!("custom path unresolved: {} ({err})", spec.raw),
    }
}

fn custom_path_scan_time_reason(err: &str) -> ScanRejection {
    ScanRejection {
        status_reason: format!("custom path not allowed at scan time: {err}"),
        diagnostic: format!("custom path rejected at scan time: {err}"),
    }
}

fn finalize_custom_target(
    mut out: TargetScanResult,
    ctx: &mut Tier3RunCtx<'_, '_>,
    rejection: ScanRejection,
) -> TargetScanResult {
    out.scanned = false;
    out.status_reason = Some(rejection.status_reason);
    out.skipped_diagnostics.push(rejection.diagnostic);
    complete_target(ctx);
    out
}

fn scan_custom_canonical_target(
    out: &mut TargetScanResult,
    canon: CanonicalPath,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<(), ScanError> {
    let td = custom_path_target_definition(out, ctx.min_age);
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

fn complete_target(ctx: &mut Tier3RunCtx<'_, '_>) {
    ctx.progress.completed_targets = ctx.progress.completed_targets.saturating_add(1);
    ctx.progress.emit(None, None);
}

fn custom_path_target_definition(out: &TargetScanResult, min_age: Duration) -> TargetDefinition {
    TargetDefinition {
        id: out.target_id.clone(),
        name: out.target_name.clone(),
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
