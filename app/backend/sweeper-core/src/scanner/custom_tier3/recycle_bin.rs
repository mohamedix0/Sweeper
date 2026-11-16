use super::Tier3RunCtx;
use super::*;

pub(super) fn append_tier3_targets(
    report: &mut ScanReport,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<(), ScanError> {
    if !ctx.settings.enable_tier3 {
        return Ok(());
    }
    report.targets.push(scan_tier3_recycle_bin(ctx)?);
    let stale = stale_projects::scan_tier3_stale_project_dirs(ctx)?;
    report.targets.extend(stale);
    Ok(())
}

fn scan_tier3_recycle_bin(ctx: &mut Tier3RunCtx<'_, '_>) -> Result<TargetScanResult, ScanError> {
    let mut result = empty_recycle_bin_result();
    emit_recycle_bin_start(ctx, &result)?;
    let valid_roots = collect_recycle_bin_items(ctx, &mut result)?;
    finalize_recycle_bin_scan(&mut result, valid_roots, ctx)
}

fn emit_recycle_bin_start(
    ctx: &mut Tier3RunCtx<'_, '_>,
    result: &TargetScanResult,
) -> Result<(), ScanError> {
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    ctx.progress
        .emit(Some(&result.target_id), Some(&result.target_name));
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    Ok(())
}

fn empty_recycle_bin_result() -> TargetScanResult {
    super::super::empty_target_result(super::super::TargetResultMeta {
        target_id: "recycle-bin".to_string(),
        target_name: "Recycle Bin (current user)".to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
    })
}

struct RecycleBinCollectCtx<'a> {
    target_id: &'a str,
    valid_roots: &'a mut Vec<(String, CanonicalPath)>,
    diagnostics: &'a mut Vec<String>,
}

fn collect_recycle_bin_items(
    ctx: &Tier3RunCtx<'_, '_>,
    result: &mut TargetScanResult,
) -> Result<Vec<(String, CanonicalPath)>, ScanError> {
    let roots = resolve_recycle_bin_roots(ctx.opts);
    let mut valid_roots = Vec::new();
    let target_id = result.target_id.clone();
    let mut collect = RecycleBinCollectCtx {
        target_id: &target_id,
        valid_roots: &mut valid_roots,
        diagnostics: &mut result.skipped_diagnostics,
    };
    for r in roots {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        collect_recycle_bin_root(r, &mut collect);
    }
    Ok(valid_roots)
}

fn resolve_recycle_bin_roots(opts: &ScanSettingsOptions) -> Vec<String> {
    if !opts.recycle_bin_roots.is_empty() {
        return opts.recycle_bin_roots.clone();
    }
    discover_recycle_bin_roots()
}

fn collect_recycle_bin_root(raw: String, ctx: &mut RecycleBinCollectCtx<'_>) {
    if raw.trim().is_empty() {
        return;
    }
    let canon = match canonicalize_recycle_bin_root(&raw) {
        Ok(v) => v,
        Err(e) => {
            ctx.diagnostics
                .push(format!("recycle-bin root unresolved: {raw} ({e})"));
            return;
        }
    };
    record_recycle_bin_root(canon, ctx);
}

fn canonicalize_recycle_bin_root(raw: &str) -> Result<CanonicalPath, String> {
    CanonicalPath::from_existing_path(raw)
        .map(|v| v.into_canonical())
        .map_err(|e| e.to_string())
}

fn record_recycle_bin_root(canon: CanonicalPath, ctx: &mut RecycleBinCollectCtx<'_>) {
    match validate_recycle_bin_root(&canon) {
        Ok(()) => ctx.valid_roots.push((ctx.target_id.to_string(), canon)),
        Err(e) => ctx
            .diagnostics
            .push(format!("recycle-bin root skipped: {} ({e})", canon)),
    }
}

fn finalize_recycle_bin_scan(
    result: &mut TargetScanResult,
    mut valid_roots: Vec<(String, CanonicalPath)>,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<TargetScanResult, ScanError> {
    if valid_roots.is_empty() {
        return Ok(finalize_empty_recycle_bin(result, ctx));
    }
    dedupe_recycle_bin_roots(&mut valid_roots);
    result.resolved_paths = valid_roots.iter().map(|(_, p)| p.to_string()).collect();
    let td = recycle_bin_target_definition(ctx.min_age);
    let mut scan_ctx = allowlist::ResolvedRootsScanCtx {
        target: &td,
        roots: &valid_roots,
        min_age: ctx.min_age,
        now: ctx.now,
        progress: ctx.progress,
    };
    allowlist::scan_resolved_roots(&mut scan_ctx, result)?;
    complete_recycle_bin(ctx);
    Ok(result.clone())
}

fn finalize_empty_recycle_bin(
    result: &mut TargetScanResult,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> TargetScanResult {
    result.scanned = false;
    result.status_reason = Some("no accessible user recycle-bin root found".to_string());
    complete_recycle_bin(ctx);
    result.clone()
}

fn dedupe_recycle_bin_roots(valid_roots: &mut Vec<(String, CanonicalPath)>) {
    valid_roots.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    valid_roots.dedup_by(|a, b| a.1.is_same_path(&b.1));
}

fn complete_recycle_bin(ctx: &mut Tier3RunCtx<'_, '_>) {
    ctx.progress.completed_targets = ctx.progress.completed_targets.saturating_add(1);
    ctx.progress.emit(None, None);
}

fn recycle_bin_target_definition(min_age: Duration) -> TargetDefinition {
    TargetDefinition {
        id: "recycle-bin".to_string(),
        name: "Recycle Bin (current user)".to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
        known_folder: KnownFolder::Windows,
        relative_subpath: String::new(),
        scope: sweeper_knowledge::Scope::Contents,
        min_age_hours: (min_age.as_secs() / 3600) as u32,
        description: "Permanently empties user-accessible Recycle Bin contents (opt-in)."
            .to_string(),
        requires_elevation: false,
    }
}

fn validate_recycle_bin_root(canon: &CanonicalPath) -> Result<(), String> {
    let verdict = policy::validate_tier3_target_root(canon.as_str());
    if !verdict.allowed {
        return Err(verdict.reason);
    }
    if allowlist::path_has_link_point_ancestor(canon)? {
        return Err("traverses link point".to_string());
    }
    Ok(())
}

fn discover_recycle_bin_roots() -> Vec<String> {
    let Some(base) = recycle_bin_base_dir() else {
        return Vec::new();
    };
    let entries = match sweeper_winfs::read_dir_entries(&base) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    collect_discovered_recycle_bin_roots(entries)
}

fn recycle_bin_base_dir() -> Option<PathBuf> {
    let system_drive = sweeper_winfs::env_var("SystemDrive").unwrap_or_else(|| "C:".to_string());
    let base = PathBuf::from(format!(
        r"{}\$Recycle.Bin",
        system_drive.trim_end_matches('\\')
    ));
    if base.exists() {
        Some(base)
    } else {
        None
    }
}

fn collect_discovered_recycle_bin_roots(entries: Vec<sweeper_winfs::DirEntry>) -> Vec<String> {
    let mut out = Vec::new();
    for ent in entries {
        push_discovered_recycle_bin_root(ent, &mut out);
    }
    out
}

fn push_discovered_recycle_bin_root(ent: sweeper_winfs::DirEntry, out: &mut Vec<String>) {
    if !is_recycle_bin_candidate_entry(&ent) {
        return;
    }
    let p = ent.path.to_string_lossy().to_string();
    let canon = match CanonicalPath::from_existing_path(&p) {
        Ok(v) => v.into_canonical(),
        Err(_) => return,
    };
    if validate_recycle_bin_root(&canon).is_ok() {
        out.push(canon.to_string());
    }
}

fn is_recycle_bin_candidate_entry(ent: &sweeper_winfs::DirEntry) -> bool {
    if !ent.is_dir {
        return false;
    }
    matches!(ent.reparse, ReparseTagClass::NotReparse)
}
