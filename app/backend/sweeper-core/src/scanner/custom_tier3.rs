use super::*;
use sweeper_winfs::{KnownFolderId, ReparseTagClass};

pub(super) fn scan_custom_path_target(
    idx: usize,
    raw_path: &str,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let out = empty_custom_path_result(idx);
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    progress.emit(Some(&out.target_id), Some(&out.target_name));
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    if let Err((reason, diag)) = validate_custom_path_for_scan(raw_path) {
        return Ok(finalize_custom_target(out, progress, reason, diag));
    }
    let canon = match resolve_custom_path_for_scan(raw_path) {
        Ok(c) => c,
        Err((reason, diag)) => {
            return Ok(finalize_custom_target(out, progress, reason, diag));
        }
    };
    scan_custom_canonical_target(out, canon, min_age, now, progress)
}

pub(super) fn empty_custom_path_result(idx: usize) -> TargetScanResult {
    super::empty_target_result(super::TargetResultMeta {
        target_id: format!("custom-path-{}", idx + 1),
        target_name: "Custom Path".to_string(),
        tier: Tier::Tier2,
        risk: Risk::Caution,
    })
}

pub(super) fn validate_custom_path_for_scan(raw_path: &str) -> Result<(), (String, String)> {
    let verdict = policy::validate_custom_path(raw_path);
    if verdict.allowed {
        return Ok(());
    }
    Err((
        format!(
            "custom path invalid/missing (no fallback): {}",
            verdict.reason
        ),
        format!("custom path rejected: {} ({})", raw_path, verdict.reason),
    ))
}

pub(super) fn resolve_custom_path_for_scan(
    raw_path: &str,
) -> Result<CanonicalPath, (String, String)> {
    let canon = CanonicalPath::from_existing_path(raw_path)
        .map(|v| v.into_canonical())
        .map_err(|e| {
            (
                format!("custom path missing/unresolvable (no fallback): {e}"),
                format!("custom path unresolved: {raw_path} ({e})"),
            )
        })?;
    super::allowlist::validate_resolved_canonical_root(&canon).map_err(|e| {
        (
            format!("custom path not allowed at scan time: {e}"),
            format!("custom path rejected at scan time: {e}"),
        )
    })?;
    Ok(canon)
}

pub(super) fn finalize_custom_target(
    mut out: TargetScanResult,
    progress: &mut ScanProgressRuntime,
    status_reason: String,
    diagnostic: String,
) -> TargetScanResult {
    out.scanned = false;
    out.status_reason = Some(status_reason);
    out.skipped_diagnostics.push(diagnostic);
    progress.completed_targets = progress.completed_targets.saturating_add(1);
    progress.emit(None, None);
    out
}

pub(super) fn scan_custom_canonical_target(
    mut out: TargetScanResult,
    canon: CanonicalPath,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let td = custom_path_target_definition(&out.target_id, &out.target_name, min_age);
    out.resolved_paths = vec![canon.to_string()];
    super::allowlist::scan_resolved_roots(
        &td,
        &[(out.target_id.clone(), canon)],
        min_age,
        now,
        &mut out,
        progress,
    )?;
    progress.completed_targets = progress.completed_targets.saturating_add(1);
    progress.emit(None, None);
    Ok(out)
}

pub(super) fn custom_path_target_definition(
    target_id: &str,
    target_name: &str,
    min_age: Duration,
) -> TargetDefinition {
    TargetDefinition {
        id: target_id.to_string(),
        name: target_name.to_string(),
        tier: Tier::Tier2,
        risk: Risk::Caution,
        known_folder: KnownFolder::LocalAppData, // unused for custom resolved scan
        relative_subpath: String::new(),
        scope: sweeper_knowledge::Scope::Contents,
        min_age_hours: (min_age.as_secs() / 3600) as u32,
        description: "User-validated custom path (contents only).".to_string(),
        requires_elevation: false,
    }
}

pub(super) fn append_custom_path_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<(), ScanError> {
    for (idx, raw) in settings.custom_paths.iter().enumerate() {
        if progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        report
            .targets
            .push(scan_custom_path_target(idx, raw, min_age, now, progress)?);
    }
    Ok(())
}

pub(super) fn append_tier3_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<(), ScanError> {
    if !settings.enable_tier3 {
        return Ok(());
    }
    report
        .targets
        .push(scan_tier3_recycle_bin(opts, min_age, now, progress)?);
    let stale = scan_tier3_stale_project_dirs(settings, opts, now, progress)?;
    report.targets.extend(stale);
    Ok(())
}

pub(super) fn scan_tier3_recycle_bin(
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    let mut result = empty_recycle_bin_result();
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    progress.emit(Some(&result.target_id), Some(&result.target_name));
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    let valid_roots = collect_recycle_bin_items(opts, &mut result, progress)?;
    finalize_recycle_bin_scan(result, valid_roots, min_age, now, progress)
}

pub(super) fn empty_recycle_bin_result() -> TargetScanResult {
    super::empty_target_result(super::TargetResultMeta {
        target_id: "recycle-bin".to_string(),
        target_name: "Recycle Bin (current user)".to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
    })
}

pub(super) struct RecycleBinCollectCtx<'a> {
    target_id: &'a str,
    valid_roots: &'a mut Vec<(String, CanonicalPath)>,
    diagnostics: &'a mut Vec<String>,
}

pub(super) fn collect_recycle_bin_items(
    opts: &ScanSettingsOptions,
    result: &mut TargetScanResult,
    progress: &ScanProgressRuntime,
) -> Result<Vec<(String, CanonicalPath)>, ScanError> {
    let roots = if !opts.recycle_bin_roots.is_empty() {
        opts.recycle_bin_roots.clone()
    } else {
        discover_recycle_bin_roots()
    };
    let mut valid_roots = Vec::new();
    let target_id = result.target_id.clone();
    let mut ctx = RecycleBinCollectCtx {
        target_id: &target_id,
        valid_roots: &mut valid_roots,
        diagnostics: &mut result.skipped_diagnostics,
    };
    for r in roots {
        if progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        collect_recycle_bin_root(r, &mut ctx);
    }
    Ok(valid_roots)
}

pub(super) fn collect_recycle_bin_root(raw: String, ctx: &mut RecycleBinCollectCtx<'_>) {
    if raw.trim().is_empty() {
        return;
    }
    let canon = match CanonicalPath::from_existing_path(&raw) {
        Ok(v) => v.into_canonical(),
        Err(e) => {
            ctx.diagnostics
                .push(format!("recycle-bin root unresolved: {raw} ({e})"));
            return;
        }
    };
    match validate_recycle_bin_root(&canon) {
        Ok(()) => ctx.valid_roots.push((ctx.target_id.to_string(), canon)),
        Err(e) => ctx
            .diagnostics
            .push(format!("recycle-bin root skipped: {} ({e})", canon)),
    }
}

pub(super) fn finalize_recycle_bin_scan(
    mut result: TargetScanResult,
    mut valid_roots: Vec<(String, CanonicalPath)>,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    if valid_roots.is_empty() {
        result.scanned = false;
        result.status_reason = Some("no accessible user recycle-bin root found".to_string());
        progress.completed_targets = progress.completed_targets.saturating_add(1);
        progress.emit(None, None);
        return Ok(result);
    }
    valid_roots.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    valid_roots.dedup_by(|a, b| a.1.is_same_path(&b.1));
    result.resolved_paths = valid_roots.iter().map(|(_, p)| p.to_string()).collect();
    let td = recycle_bin_target_definition(min_age);
    super::allowlist::scan_resolved_roots(&td, &valid_roots, min_age, now, &mut result, progress)?;
    progress.completed_targets = progress.completed_targets.saturating_add(1);
    progress.emit(None, None);
    Ok(result)
}

pub(super) fn recycle_bin_target_definition(min_age: Duration) -> TargetDefinition {
    TargetDefinition {
        id: "recycle-bin".to_string(),
        name: "Recycle Bin (current user)".to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
        known_folder: KnownFolder::Windows, // unused
        relative_subpath: String::new(),
        scope: sweeper_knowledge::Scope::Contents,
        min_age_hours: (min_age.as_secs() / 3600) as u32,
        description: "Permanently empties user-accessible Recycle Bin contents (opt-in)."
            .to_string(),
        requires_elevation: false,
    }
}

pub(super) fn validate_recycle_bin_root(canon: &CanonicalPath) -> Result<(), String> {
    let verdict = policy::validate_tier3_target_root(canon.as_str());
    if !verdict.allowed {
        return Err(verdict.reason);
    }
    if super::allowlist::path_has_link_point_ancestor(canon)? {
        return Err("traverses link point".to_string());
    }
    Ok(())
}

pub(super) fn discover_recycle_bin_roots() -> Vec<String> {
    let mut out = Vec::new();
    let system_drive = sweeper_winfs::env_var("SystemDrive").unwrap_or_else(|| "C:".to_string());
    let base = PathBuf::from(format!(
        r"{}\$Recycle.Bin",
        system_drive.trim_end_matches('\\')
    ));
    if !base.exists() {
        return out;
    }
    let entries = match sweeper_winfs::read_dir_entries(&base) {
        Ok(v) => v,
        Err(_) => return out,
    };
    for ent in entries {
        if !ent.is_dir {
            continue;
        }
        if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
            continue;
        }
        let p = ent.path.to_string_lossy().to_string();
        if let Ok(v) = CanonicalPath::from_existing_path(&p) {
            let canon = v.into_canonical();
            // Prefer SID-like subfolders (S-1-*) when discoverable; still only include handle-accessible safe ones.
            let name = ent.name.to_string_lossy();
            let sid_like = name.starts_with("S-1-");
            if !sid_like {
                // Allow non-SID named dirs only if accessible and safe (best-effort); keep conservative.
            }
            if validate_recycle_bin_root(&canon).is_ok() {
                out.push(canon.to_string());
            }
        }
    }
    out
}

pub(super) type StaleDirInfo = (String, u64, u64, u64, u64, u64);

pub(super) struct StaleScanCtx<'a> {
    stale_threshold: Duration,
    now: SystemTime,
    progress: &'a mut ScanProgressRuntime,
}

pub(super) struct StaleDirs<'a> {
    rust_dirs: &'a mut Vec<StaleDirInfo>,
    node_dirs: &'a mut Vec<StaleDirInfo>,
}

pub(super) fn scan_tier3_stale_project_dirs(
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<Vec<TargetScanResult>, ScanError> {
    let search_roots = if !opts.tier3_project_search_roots.is_empty() {
        opts.tier3_project_search_roots.clone()
    } else {
        default_tier3_project_search_roots()
    };

    let stale_days = settings.stale_project_days;
    let stale_threshold = Duration::from_secs(u64::from(stale_days) * 24 * 3600);
    let mut rust_dirs: Vec<StaleDirInfo> = Vec::new();
    let mut node_dirs: Vec<StaleDirInfo> = Vec::new();
    let mut dirs = StaleDirs {
        rust_dirs: &mut rust_dirs,
        node_dirs: &mut node_dirs,
    };
    let mut ctx = StaleScanCtx {
        stale_threshold,
        now,
        progress,
    };

    for root in search_roots {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        discover_stale_project_dirs(&root, &mut ctx, &mut dirs)?;
    }

    let mut out = Vec::new();
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    progress.emit(Some("stale-rust-target"), Some("Stale Rust target/"));
    out.push(build_stale_project_target_result(
        "stale-rust-target",
        "Stale Rust target/",
        rust_dirs,
    ));
    progress.completed_targets = progress.completed_targets.saturating_add(1);
    progress.emit(None, None);
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }

    progress.emit(Some("stale-node-modules"), Some("Stale node_modules/"));
    out.push(build_stale_project_target_result(
        "stale-node-modules",
        "Stale node_modules/",
        node_dirs,
    ));
    progress.completed_targets = progress.completed_targets.saturating_add(1);
    progress.emit(None, None);
    Ok(out)
}

pub(super) fn default_tier3_project_search_roots() -> Vec<String> {
    let mut roots = Vec::new();
    if let Ok(p) = sweeper_winfs::resolve_known_folder(KnownFolderId::UserProfile) {
        if let Ok(v) = CanonicalPath::from_existing_path(&p.to_string_lossy()) {
            let canon = v.into_canonical();
            if !policy::is_subtree_protected(&canon)
                && !policy::is_ancestor_of_subtree_protected(&canon)
                && !policy::is_protected_container_root(&canon)
            {
                roots.push(canon.to_string());
            }
        }
    }
    roots
}

pub(super) struct StaleWalkState {
    stack: Vec<PathBuf>,
    seen: std::collections::HashSet<String>,
}

pub(super) fn discover_stale_project_dirs(
    search_root: &str,
    ctx: &mut StaleScanCtx<'_>,
    dirs: &mut StaleDirs<'_>,
) -> Result<(), ScanError> {
    let Some(root) = normalize_stale_search_root(search_root) else {
        return Ok(());
    };
    walk_stale_search_root(root, ctx, dirs)?;
    finalize_stale_dir_lists(dirs.rust_dirs, dirs.node_dirs);
    Ok(())
}

pub(super) fn normalize_stale_search_root(search_root: &str) -> Option<PathBuf> {
    let root = CanonicalPath::from_existing_path(search_root)
        .ok()?
        .into_canonical();
    if is_disallowed_stale_search_root(&root) {
        return None;
    }
    Some(PathBuf::from(root.as_str()))
}

pub(super) fn is_disallowed_stale_search_root(root: &CanonicalPath) -> bool {
    policy::is_subtree_protected(root)
        || policy::is_ancestor_of_subtree_protected(root)
        || policy::is_protected_container_root(root)
}

pub(super) fn walk_stale_search_root(
    root: PathBuf,
    ctx: &mut StaleScanCtx<'_>,
    dirs: &mut StaleDirs<'_>,
) -> Result<(), ScanError> {
    let mut state = StaleWalkState {
        stack: vec![root],
        seen: std::collections::HashSet::new(),
    };
    while let Some(dir) = state.stack.pop() {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        process_stale_search_dir(&dir, ctx, dirs, &mut state)?;
    }
    Ok(())
}

pub(super) fn process_stale_search_dir(
    dir: &Path,
    ctx: &mut StaleScanCtx<'_>,
    dirs: &mut StaleDirs<'_>,
    state: &mut StaleWalkState,
) -> Result<(), ScanError> {
    if !mark_stale_dir_seen(dir, &mut state.seen) || is_disallowed_tier3_scan_path(dir) {
        return Ok(());
    }
    let Ok(entries) = sweeper_winfs::read_dir_entries(dir) else {
        return Ok(());
    };
    collect_stale_artifacts(&entries, ctx, dirs)?;
    push_stale_traversal_children(entries, &mut state.stack);
    Ok(())
}

pub(super) fn mark_stale_dir_seen(
    dir: &Path,
    seen: &mut std::collections::HashSet<String>,
) -> bool {
    let key = dir.to_string_lossy().to_ascii_uppercase();
    seen.insert(key)
}

pub(super) fn collect_stale_artifacts(
    entries: &[sweeper_winfs::DirEntry],
    ctx: &mut StaleScanCtx<'_>,
    dirs: &mut StaleDirs<'_>,
) -> Result<(), ScanError> {
    for ent in entries {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        record_stale_artifact_if_eligible(ent, ctx, dirs)?;
    }
    Ok(())
}

pub(super) fn record_stale_artifact_if_eligible(
    ent: &sweeper_winfs::DirEntry,
    ctx: &mut StaleScanCtx<'_>,
    dirs: &mut StaleDirs<'_>,
) -> Result<(), ScanError> {
    if !is_plain_directory_entry(ent) {
        return Ok(());
    }
    let name = ent.name.to_string_lossy();
    if name.eq_ignore_ascii_case("target") {
        append_stale_dir_info(stale_rust_target_info(&ent.path, ctx)?, dirs.rust_dirs);
        return Ok(());
    }
    if name.eq_ignore_ascii_case("node_modules") {
        append_stale_dir_info(stale_node_modules_info(&ent.path, ctx)?, dirs.node_dirs);
    }
    Ok(())
}

pub(super) fn is_plain_directory_entry(ent: &sweeper_winfs::DirEntry) -> bool {
    ent.is_dir && matches!(ent.reparse, sweeper_winfs::ReparseTagClass::NotReparse)
}

pub(super) fn append_stale_dir_info(info: Option<StaleDirInfo>, out: &mut Vec<StaleDirInfo>) {
    if let Some(v) = info {
        out.push(v);
    }
}

pub(super) fn push_stale_traversal_children(
    entries: Vec<sweeper_winfs::DirEntry>,
    stack: &mut Vec<PathBuf>,
) {
    for ent in entries {
        if should_traverse_stale_child(&ent) {
            stack.push(ent.path);
        }
    }
}

pub(super) fn should_traverse_stale_child(ent: &sweeper_winfs::DirEntry) -> bool {
    if !is_plain_directory_entry(ent) {
        return false;
    }
    if is_stale_traversal_skip_name(&ent.name.to_string_lossy(), &ent.path) {
        return false;
    }
    !is_disallowed_tier3_scan_path(&ent.path)
}

pub(super) fn is_stale_traversal_skip_name(name: &str, path: &Path) -> bool {
    if is_known_stale_skip_name(name) {
        return true;
    }
    if name.eq_ignore_ascii_case("target") && path.join("..\\Cargo.toml").exists() {
        return true;
    }
    name.eq_ignore_ascii_case("node_modules")
}

pub(super) fn is_known_stale_skip_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("$Recycle.Bin")
        || name.eq_ignore_ascii_case("System Volume Information")
        || name.eq_ignore_ascii_case("Windows")
        || name.eq_ignore_ascii_case("Program Files")
        || name.eq_ignore_ascii_case("Program Files (x86)")
}

pub(super) fn finalize_stale_dir_lists(
    rust_dirs: &mut Vec<StaleDirInfo>,
    node_dirs: &mut Vec<StaleDirInfo>,
) {
    rust_dirs.sort_by(|a, b| a.0.cmp(&b.0));
    rust_dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    node_dirs.sort_by(|a, b| a.0.cmp(&b.0));
    node_dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
}

pub(super) fn is_disallowed_tier3_scan_path(path: &Path) -> bool {
    let Ok(canon) = CanonicalPath::parse_lexical(&path.to_string_lossy()) else {
        return true;
    };
    if policy::is_subtree_protected(&canon)
        || policy::is_ancestor_of_subtree_protected(&canon)
        || policy::is_protected_container_root(&canon)
    {
        return true;
    }
    // Extra hard blocks by name/path anchors.
    let s = canon.as_str().to_ascii_uppercase();
    s.starts_with(r"C:\WINDOWS")
        || s.starts_with(r"C:\PROGRAM FILES")
        || s.starts_with(r"C:\PROGRAM FILES (X86)")
        || s.contains(r"\SYSTEM VOLUME INFORMATION")
}

pub(super) fn stale_rust_target_info(
    target_dir: &Path,
    ctx: &mut StaleScanCtx<'_>,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Some(parent) = target_dir.parent() else {
        return Ok(None);
    };
    let cargo_toml = parent.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return Ok(None);
    }
    if !is_path_stale_enough(&cargo_toml, ctx.stale_threshold, ctx.now) {
        return Ok(None);
    }
    stale_dir_item_info(
        target_dir,
        ctx.progress,
        Some("stale-rust-target"),
        Some("Stale Rust target/"),
    )
}

pub(super) fn stale_node_modules_info(
    node_modules_dir: &Path,
    ctx: &mut StaleScanCtx<'_>,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Some(parent) = node_modules_dir.parent() else {
        return Ok(None);
    };
    let lockfiles = [
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
    ];
    let mut found = false;
    for lf in lockfiles {
        let p = parent.join(lf);
        if p.is_file() {
            found = true;
            if !is_path_stale_enough(&p, ctx.stale_threshold, ctx.now) {
                return Ok(None);
            }
        }
    }
    if !found {
        return Ok(None);
    }
    stale_dir_item_info(
        node_modules_dir,
        ctx.progress,
        Some("stale-node-modules"),
        Some("Stale node_modules/"),
    )
}

pub(super) fn is_path_stale_enough(
    path: &Path,
    stale_threshold: Duration,
    now: SystemTime,
) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let mtime = match meta.modified() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if mtime > now {
        return false;
    }
    now.duration_since(mtime)
        .map(|age| age >= stale_threshold)
        .unwrap_or(false)
}

pub(super) fn stale_dir_item_info(
    dir: &Path,
    progress: &mut ScanProgressRuntime,
    target_id: Option<&str>,
    target_name: Option<&str>,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Ok(v) = CanonicalPath::from_existing_path(&dir.to_string_lossy()) else {
        return Ok(None);
    };
    let canon = v.into_canonical();
    // Safety: must be eligible Tier 3 root and not protected/link point.
    if !policy::validate_tier3_target_root(canon.as_str()).allowed {
        return Ok(None);
    }
    match summarize_directory_for_scan(dir, progress, target_id, target_name) {
        Ok((total, eligible, count, eligible_count, mtime)) => Ok(Some((
            canon.to_string(),
            total,
            eligible,
            count,
            eligible_count,
            mtime,
        ))),
        Err(ScanError::Cancelled) => Err(ScanError::Cancelled),
        Err(_) => Ok(None),
    }
}

pub(super) fn summarize_directory_for_scan(
    dir: &Path,
    progress: &mut ScanProgressRuntime,
    target_id: Option<&str>,
    target_name: Option<&str>,
) -> Result<(u64, u64, u64, u64, u64), ScanError> {
    let mut total: u64 = 0;
    let mut eligible: u64 = 0;
    let mut count: u64 = 0;
    let mut eligible_count: u64 = 0;
    let mut max_mtime: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        let entries =
            sweeper_winfs::read_dir_entries(&d).map_err(|e| ScanError::Targets(e.to_string()))?;
        for ent in entries {
            if progress.cancelled() {
                return Err(ScanError::Cancelled);
            }
            match ent.reparse {
                ReparseTagClass::NotReparse => {}
                _ => continue, // do not traverse/count skipped reparse entries in aggregate
            }
            if ent.is_dir {
                stack.push(ent.path);
            } else {
                let mt = super::allowlist::filetime_to_unix_secs(ent.last_write_time);
                max_mtime = max_mtime.max(mt);
                total = total.saturating_add(ent.size);
                eligible = eligible.saturating_add(ent.size); // staleness decided by project marker; treat contents as eligible
                count = count.saturating_add(1);
                eligible_count = eligible_count.saturating_add(1);
                progress.scanned_items = progress.scanned_items.saturating_add(1);
                progress.scanned_bytes = progress.scanned_bytes.saturating_add(ent.size);
                progress.maybe_emit_throttled(target_id, target_name);
            }
        }
        progress.maybe_emit_throttled(target_id, target_name);
    }
    Ok((total, eligible, count, eligible_count, max_mtime))
}

pub(super) fn build_stale_project_target_result(
    target_id: &str,
    target_name: &str,
    dirs: Vec<StaleDirInfo>,
) -> TargetScanResult {
    let mut result = super::empty_target_result(super::TargetResultMeta {
        target_id: target_id.to_string(),
        target_name: target_name.to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
    });
    if dirs.is_empty() {
        result.scanned = false;
        result.status_reason = Some("no stale directories found".to_string());
        return result;
    }
    result.scanned = true;
    for (i, (path, total, eligible, count, eligible_count, mtime)) in dirs.into_iter().enumerate() {
        result.resolved_paths.push(path.clone());
        result.total_bytes = result.total_bytes.saturating_add(total);
        result.eligible_bytes = result.eligible_bytes.saturating_add(eligible);
        result.file_count = result.file_count.saturating_add(count);
        result.eligible_count = result.eligible_count.saturating_add(eligible_count);
        result.items.push(ScanItem {
            id: format!("{}::{}", target_id, i + 1),
            target_id: target_id.to_string(),
            path,
            file_identity: None,
            size: total,
            mtime_unix_secs: mtime,
            age_eligible: true,
        });
    }
    result
}
