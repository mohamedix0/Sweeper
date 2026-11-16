use super::stale_policy;
use super::Tier3RunCtx;
use super::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

type StaleDirInfo = (String, u64, u64, u64, u64, u64);

#[derive(Clone, Copy)]
enum StaleKind {
    RustTarget,
    NodeModules,
}

impl StaleKind {
    const fn spec(self) -> StaleKindSpec {
        match self {
            Self::RustTarget => StaleKindSpec {
                kind: self,
                target_id: "stale-rust-target",
                target_name: "Stale Rust target/",
            },
            Self::NodeModules => StaleKindSpec {
                kind: self,
                target_id: "stale-node-modules",
                target_name: "Stale node_modules/",
            },
        }
    }
}

#[derive(Clone, Copy)]
struct StaleKindSpec {
    kind: StaleKind,
    target_id: &'static str,
    target_name: &'static str,
}

struct StaleScanState<'c, 'a, 'p> {
    ctx: &'c mut Tier3RunCtx<'a, 'p>,
    stale_threshold: Duration,
    rust_dirs: Vec<StaleDirInfo>,
    node_dirs: Vec<StaleDirInfo>,
}

struct StaleWalk<'a> {
    stack: Vec<PathBuf>,
    seen: &'a mut HashSet<String>,
}

pub(super) fn scan_tier3_stale_project_dirs(
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<Vec<TargetScanResult>, ScanError> {
    let search_roots = stale_policy::resolve_stale_search_roots(ctx.opts);
    let stale_threshold =
        Duration::from_secs(u64::from(ctx.settings.stale_project_days) * 24 * 3600);
    let mut state = StaleScanState {
        stale_threshold,
        rust_dirs: Vec::new(),
        node_dirs: Vec::new(),
        ctx,
    };
    for root in search_roots {
        if state.ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        discover_stale_project_dirs(&root, &mut state)?;
    }
    build_stale_target_results(state)
}

fn build_stale_target_results(
    mut state: StaleScanState<'_, '_, '_>,
) -> Result<Vec<TargetScanResult>, ScanError> {
    finalize_stale_dir_lists(&mut state.rust_dirs, &mut state.node_dirs);
    let rust = state.rust_dirs;
    let node = state.node_dirs;
    Ok(vec![
        emit_stale_target(StaleKind::RustTarget.spec(), rust, state.ctx)?,
        emit_stale_target(StaleKind::NodeModules.spec(), node, state.ctx)?,
    ])
}

fn emit_stale_target(
    spec: StaleKindSpec,
    dirs: Vec<StaleDirInfo>,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<TargetScanResult, ScanError> {
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    ctx.progress
        .emit(Some(spec.target_id), Some(spec.target_name));
    let result = build_stale_project_target_result(spec, dirs);
    ctx.progress.completed_targets = ctx.progress.completed_targets.saturating_add(1);
    ctx.progress.emit(None, None);
    Ok(result)
}

fn discover_stale_project_dirs(
    search_root: &str,
    state: &mut StaleScanState<'_, '_, '_>,
) -> Result<(), ScanError> {
    let Some(root) = stale_policy::normalize_stale_search_root(search_root) else {
        return Ok(());
    };
    let mut seen = HashSet::new();
    let mut walk = StaleWalk {
        stack: vec![root],
        seen: &mut seen,
    };
    while let Some(dir) = walk.stack.pop() {
        if state.ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        process_stale_search_dir(&dir, state, &mut walk)?;
    }
    Ok(())
}

fn process_stale_search_dir(
    dir: &Path,
    state: &mut StaleScanState<'_, '_, '_>,
    walk: &mut StaleWalk<'_>,
) -> Result<(), ScanError> {
    if !should_visit_stale_search_dir(dir, walk) {
        return Ok(());
    }
    let Ok(entries) = sweeper_winfs::read_dir_entries(dir) else {
        return Ok(());
    };
    scan_stale_dir_entries(&entries, state)?;
    enqueue_stale_child_dirs(entries, walk);
    Ok(())
}

fn should_visit_stale_search_dir(dir: &Path, walk: &mut StaleWalk<'_>) -> bool {
    let key = dir.to_string_lossy().to_ascii_uppercase();
    if !walk.seen.insert(key) {
        return false;
    }
    !stale_policy::is_disallowed_tier3_scan_path(dir)
}

fn scan_stale_dir_entries(
    entries: &[sweeper_winfs::DirEntry],
    state: &mut StaleScanState<'_, '_, '_>,
) -> Result<(), ScanError> {
    for ent in entries {
        if state.ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        collect_stale_artifact(ent, state)?;
    }
    Ok(())
}

fn enqueue_stale_child_dirs(entries: Vec<sweeper_winfs::DirEntry>, walk: &mut StaleWalk<'_>) {
    for ent in entries {
        if stale_policy::should_traverse_stale_child(&ent) {
            walk.stack.push(ent.path);
        }
    }
}

fn collect_stale_artifact(
    ent: &sweeper_winfs::DirEntry,
    state: &mut StaleScanState<'_, '_, '_>,
) -> Result<(), ScanError> {
    if !stale_policy::is_plain_directory_entry(ent) {
        return Ok(());
    }
    let name = ent.name.to_string_lossy();
    if name.eq_ignore_ascii_case("target") {
        record_stale_dir(StaleKind::RustTarget.spec(), &ent.path, state)?;
    } else if name.eq_ignore_ascii_case("node_modules") {
        record_stale_dir(StaleKind::NodeModules.spec(), &ent.path, state)?;
    }
    Ok(())
}

fn record_stale_dir(
    spec: StaleKindSpec,
    dir: &Path,
    state: &mut StaleScanState<'_, '_, '_>,
) -> Result<(), ScanError> {
    if let Some(info) = stale_dir_info(spec, dir, state)? {
        match spec.kind {
            StaleKind::RustTarget => state.rust_dirs.push(info),
            StaleKind::NodeModules => state.node_dirs.push(info),
        }
    }
    Ok(())
}

fn stale_dir_info(
    spec: StaleKindSpec,
    dir: &Path,
    state: &mut StaleScanState<'_, '_, '_>,
) -> Result<Option<StaleDirInfo>, ScanError> {
    let Some(parent) = dir.parent() else {
        return Ok(None);
    };
    let stale = match spec.kind {
        StaleKind::RustTarget => stale_policy::is_stale_rust_target_dir(
            dir,
            parent,
            state.stale_threshold,
            state.ctx.now,
        ),
        StaleKind::NodeModules => {
            stale_policy::is_stale_node_modules_dir(parent, state.stale_threshold, state.ctx.now)
        }
    };
    if !stale {
        return Ok(None);
    }
    stale_dir_item_info(spec, dir, state.ctx)
}

fn stale_dir_item_info(
    spec: StaleKindSpec,
    dir: &Path,
    ctx: &mut Tier3RunCtx<'_, '_>,
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
        ctx.progress,
        Some(spec.target_id),
        Some(spec.target_name),
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

fn finalize_stale_dir_lists(rust_dirs: &mut Vec<StaleDirInfo>, node_dirs: &mut Vec<StaleDirInfo>) {
    sort_dedupe_stale_dirs(rust_dirs);
    sort_dedupe_stale_dirs(node_dirs);
}

fn sort_dedupe_stale_dirs(dirs: &mut Vec<StaleDirInfo>) {
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
}

fn build_stale_project_target_result(
    spec: StaleKindSpec,
    dirs: Vec<StaleDirInfo>,
) -> TargetScanResult {
    let mut result = super::super::empty_target_result(super::super::TargetResultMeta {
        target_id: spec.target_id.to_string(),
        target_name: spec.target_name.to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
    });
    if dirs.is_empty() {
        result.scanned = false;
        result.status_reason = Some("no stale directories found".to_string());
        return result;
    }
    result.scanned = true;
    for (idx, (path, total, eligible, count, eligible_count, mtime)) in dirs.into_iter().enumerate()
    {
        result.resolved_paths.push(path.clone());
        result.total_bytes = result.total_bytes.saturating_add(total);
        result.eligible_bytes = result.eligible_bytes.saturating_add(eligible);
        result.file_count = result.file_count.saturating_add(count);
        result.eligible_count = result.eligible_count.saturating_add(eligible_count);
        result.items.push(ScanItem {
            id: format!("{}::{}", spec.target_id, idx + 1),
            target_id: spec.target_id.to_string(),
            path,
            file_identity: None,
            size: total,
            mtime_unix_secs: mtime,
            age_eligible: true,
        });
    }
    result
}
