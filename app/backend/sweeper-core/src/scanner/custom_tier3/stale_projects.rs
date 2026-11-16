use super::stale_policy;
use super::CustomScanCtx;
use super::*;
use std::path::{Path, PathBuf};

pub(super) type StaleDirInfo = (String, u64, u64, u64, u64, u64);

struct StaleScanCtx<'a, 'p> {
    stale_threshold: Duration,
    now: SystemTime,
    progress: &'p mut ScanProgressRuntime,
    _marker: std::marker::PhantomData<&'a ()>,
}

struct StaleDirs {
    rust_dirs: Vec<StaleDirInfo>,
    node_dirs: Vec<StaleDirInfo>,
}

struct StaleWalkState {
    stack: Vec<PathBuf>,
    seen: std::collections::HashSet<String>,
}

pub(super) fn scan_tier3_stale_project_dirs(
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    ctx: &mut CustomScanCtx<'_>,
) -> Result<Vec<TargetScanResult>, ScanError> {
    let search_roots = stale_policy::resolve_stale_search_roots(opts);
    let mut dirs = StaleDirs {
        rust_dirs: Vec::new(),
        node_dirs: Vec::new(),
    };
    let mut stale_ctx = StaleScanCtx {
        stale_threshold: Duration::from_secs(u64::from(settings.stale_project_days) * 24 * 3600),
        now: ctx.now,
        progress: ctx.progress,
        _marker: std::marker::PhantomData,
    };
    for root in search_roots {
        if stale_ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        discover_stale_project_dirs(&root, &mut stale_ctx, &mut dirs)?;
    }
    build_stale_target_results(dirs, stale_ctx.progress)
}

fn build_stale_target_results(
    dirs: StaleDirs,
    progress: &mut ScanProgressRuntime,
) -> Result<Vec<TargetScanResult>, ScanError> {
    Ok(vec![
        emit_stale_target(
            "stale-rust-target",
            "Stale Rust target/",
            dirs.rust_dirs,
            progress,
        )?,
        emit_stale_target(
            "stale-node-modules",
            "Stale node_modules/",
            dirs.node_dirs,
            progress,
        )?,
    ])
}

fn emit_stale_target(
    target_id: &str,
    target_name: &str,
    dirs: Vec<StaleDirInfo>,
    progress: &mut ScanProgressRuntime,
) -> Result<TargetScanResult, ScanError> {
    if progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    progress.emit(Some(target_id), Some(target_name));
    let result = build_stale_project_target_result(target_id, target_name, dirs);
    progress.completed_targets = progress.completed_targets.saturating_add(1);
    progress.emit(None, None);
    Ok(result)
}

fn discover_stale_project_dirs(
    search_root: &str,
    ctx: &mut StaleScanCtx<'_, '_>,
    dirs: &mut StaleDirs,
) -> Result<(), ScanError> {
    let Some(root) = stale_policy::normalize_stale_search_root(search_root) else {
        return Ok(());
    };
    walk_stale_search_root(root, ctx, dirs)?;
    finalize_stale_dir_lists(&mut dirs.rust_dirs, &mut dirs.node_dirs);
    Ok(())
}

fn walk_stale_search_root(
    root: PathBuf,
    ctx: &mut StaleScanCtx<'_, '_>,
    dirs: &mut StaleDirs,
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

fn process_stale_search_dir(
    dir: &Path,
    ctx: &mut StaleScanCtx<'_, '_>,
    dirs: &mut StaleDirs,
    state: &mut StaleWalkState,
) -> Result<(), ScanError> {
    if !mark_stale_dir_seen(dir, &mut state.seen) {
        return Ok(());
    }
    if stale_policy::is_disallowed_tier3_scan_path(dir) {
        return Ok(());
    }
    let Ok(entries) = sweeper_winfs::read_dir_entries(dir) else {
        return Ok(());
    };
    collect_stale_artifacts(&entries, ctx, dirs)?;
    push_stale_traversal_children(entries, &mut state.stack);
    Ok(())
}

fn mark_stale_dir_seen(dir: &Path, seen: &mut std::collections::HashSet<String>) -> bool {
    let key = dir.to_string_lossy().to_ascii_uppercase();
    seen.insert(key)
}

fn collect_stale_artifacts(
    entries: &[sweeper_winfs::DirEntry],
    ctx: &mut StaleScanCtx<'_, '_>,
    dirs: &mut StaleDirs,
) -> Result<(), ScanError> {
    for ent in entries {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        record_stale_artifact_if_eligible(ent, ctx, dirs)?;
    }
    Ok(())
}

fn record_stale_artifact_if_eligible(
    ent: &sweeper_winfs::DirEntry,
    ctx: &mut StaleScanCtx<'_, '_>,
    dirs: &mut StaleDirs,
) -> Result<(), ScanError> {
    if !stale_policy::is_plain_directory_entry(ent) {
        return Ok(());
    }
    let name = ent.name.to_string_lossy();
    if name.eq_ignore_ascii_case("target") {
        let info = stale_rust_target_info(&ent.path, ctx)?;
        append_stale_dir_info(info, &mut dirs.rust_dirs);
        return Ok(());
    }
    if name.eq_ignore_ascii_case("node_modules") {
        let info = stale_node_modules_info(&ent.path, ctx)?;
        append_stale_dir_info(info, &mut dirs.node_dirs);
    }
    Ok(())
}

fn append_stale_dir_info(info: Option<StaleDirInfo>, out: &mut Vec<StaleDirInfo>) {
    if let Some(v) = info {
        out.push(v);
    }
}

fn push_stale_traversal_children(entries: Vec<sweeper_winfs::DirEntry>, stack: &mut Vec<PathBuf>) {
    for ent in entries {
        if stale_policy::should_traverse_stale_child(&ent) {
            stack.push(ent.path);
        }
    }
}

fn finalize_stale_dir_lists(rust_dirs: &mut Vec<StaleDirInfo>, node_dirs: &mut Vec<StaleDirInfo>) {
    sort_dedupe_stale_dirs(rust_dirs);
    sort_dedupe_stale_dirs(node_dirs);
}

fn sort_dedupe_stale_dirs(dirs: &mut Vec<StaleDirInfo>) {
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
}

fn stale_rust_target_info(
    target_dir: &Path,
    ctx: &mut StaleScanCtx<'_, '_>,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Some(parent) = target_dir.parent() else {
        return Ok(None);
    };
    let stale =
        stale_policy::is_stale_rust_target_dir(target_dir, parent, ctx.stale_threshold, ctx.now);
    if !stale {
        return Ok(None);
    }
    stale_dir_item_info(
        target_dir,
        ctx.progress,
        "stale-rust-target",
        "Stale Rust target/",
    )
}

fn stale_node_modules_info(
    node_modules_dir: &Path,
    ctx: &mut StaleScanCtx<'_, '_>,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Some(parent) = node_modules_dir.parent() else {
        return Ok(None);
    };
    let stale = stale_policy::is_stale_node_modules_dir(parent, ctx.stale_threshold, ctx.now);
    if !stale {
        return Ok(None);
    }
    stale_dir_item_info(
        node_modules_dir,
        ctx.progress,
        "stale-node-modules",
        "Stale node_modules/",
    )
}

fn stale_dir_item_info(
    dir: &Path,
    progress: &mut ScanProgressRuntime,
    target_id: &str,
    target_name: &str,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Ok(v) = CanonicalPath::from_existing_path(&dir.to_string_lossy()) else {
        return Ok(None);
    };
    let canon = v.into_canonical();
    if !policy::validate_tier3_target_root(canon.as_str()).allowed {
        return Ok(None);
    }
    let s = super::dir_summary::summarize_directory_for_scan(
        dir,
        progress,
        Some(target_id),
        Some(target_name),
    )?;
    Ok(Some((
        canon.to_string(),
        s.total,
        s.eligible,
        s.count,
        s.eligible_count,
        s.max_mtime,
    )))
}

fn build_stale_project_target_result(
    target_id: &str,
    target_name: &str,
    dirs: Vec<StaleDirInfo>,
) -> TargetScanResult {
    let mut result = super::super::empty_target_result(super::super::TargetResultMeta {
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
    append_stale_dir_items(&mut result, target_id, dirs);
    result
}

struct StaleScanItemInput {
    target_id: String,
    idx: usize,
    path: String,
    total: u64,
    mtime: u64,
}

fn append_stale_dir_items(result: &mut TargetScanResult, target_id: &str, dirs: Vec<StaleDirInfo>) {
    for (i, (path, total, eligible, count, eligible_count, mtime)) in dirs.into_iter().enumerate() {
        result.resolved_paths.push(path.clone());
        result.total_bytes = result.total_bytes.saturating_add(total);
        result.eligible_bytes = result.eligible_bytes.saturating_add(eligible);
        result.file_count = result.file_count.saturating_add(count);
        result.eligible_count = result.eligible_count.saturating_add(eligible_count);
        result.items.push(stale_scan_item(StaleScanItemInput {
            target_id: target_id.to_string(),
            idx: i + 1,
            path,
            total,
            mtime,
        }));
    }
}

fn stale_scan_item(input: StaleScanItemInput) -> ScanItem {
    ScanItem {
        id: format!("{}::{}", input.target_id, input.idx),
        target_id: input.target_id,
        path: input.path,
        file_identity: None,
        size: input.total,
        mtime_unix_secs: input.mtime,
        age_eligible: true,
    }
}
