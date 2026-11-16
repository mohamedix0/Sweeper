use super::*;

pub(super) struct ScanWalkCtx<'a> {
    target: &'a TargetDefinition,
    min_age: Duration,
    now: SystemTime,
    out: &'a mut TargetScanResult,
    stack: &'a mut Vec<PathBuf>,
    next_item_seq: &'a mut u64,
    progress: &'a mut ScanProgressRuntime,
}

pub(super) struct RootWalk<'a> {
    pub target: &'a TargetDefinition,
    pub root: &'a CanonicalPath,
    pub min_age: Duration,
    pub now: SystemTime,
    pub out: &'a mut TargetScanResult,
    pub next_item_seq: &'a mut u64,
    pub progress: &'a mut ScanProgressRuntime,
}

pub(super) fn scan_one_root_iterative(req: &mut RootWalk<'_>) -> Result<(), ScanError> {
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(req.root.as_str())];
    let mut ctx = ScanWalkCtx {
        target: req.target,
        min_age: req.min_age,
        now: req.now,
        out: req.out,
        stack: &mut stack,
        next_item_seq: req.next_item_seq,
        progress: req.progress,
    };
    while let Some(dir) = ctx.stack.pop() {
        scan_directory_in_walk(&mut ctx, &dir)?;
    }
    Ok(())
}

pub(super) fn scan_directory_in_walk(
    ctx: &mut ScanWalkCtx<'_>,
    dir: &Path,
) -> Result<(), ScanError> {
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    let entries = match sweeper_winfs::read_dir_entries(dir) {
        Ok(v) => v,
        Err(e) => {
            record_unreadable_directory(ctx, dir, &e);
            return Ok(());
        }
    };
    process_dir_entry_batch(ctx, entries)
}

pub(super) fn record_unreadable_directory(
    ctx: &mut ScanWalkCtx<'_>,
    dir: &Path,
    e: &dyn std::fmt::Display,
) {
    bump_skip_with_context(
        ctx.out,
        SkipReason::Unreadable,
        format!("read_dir_entries {}: {e}", dir.to_string_lossy()),
    );
    ctx.progress
        .maybe_emit_throttled(Some(ctx.target.id.as_str()), Some(ctx.target.name.as_str()));
}

pub(super) fn process_dir_entry_batch(
    ctx: &mut ScanWalkCtx<'_>,
    entries: Vec<DirEntry>,
) -> Result<(), ScanError> {
    for ent in entries {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        process_entry(ctx, &ent);
        emit_batch_progress(ctx);
    }
    emit_batch_progress(ctx);
    Ok(())
}

pub(super) fn emit_batch_progress(ctx: &mut ScanWalkCtx<'_>) {
    ctx.progress
        .maybe_emit_throttled(Some(ctx.target.id.as_str()), Some(ctx.target.name.as_str()));
}

pub(super) fn process_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    process_directory_entry(ctx, ent);
}

pub(super) fn process_directory_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    if let Some(reason) = skip_reason_for_reparse(ent.reparse) {
        bump_skip(ctx.out, reason);
        return;
    }
    if ent.is_dir {
        handle_directory_entry(ctx, ent);
        return;
    }
    record_file_entry(ctx, ent);
}

pub(super) fn skip_reason_for_reparse(reparse: ReparseTagClass) -> Option<SkipReason> {
    match reparse {
        ReparseTagClass::LinkPoint => Some(SkipReason::LinkPoint),
        ReparseTagClass::CloudPlaceholder => Some(SkipReason::CloudPlaceholder),
        ReparseTagClass::OtherReparse(_) => Some(SkipReason::OtherReparse),
        ReparseTagClass::NotReparse => None,
    }
}

pub(super) fn handle_directory_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    match inspect_directory_reparse(ent) {
        Ok(None) => ctx.stack.push(ent.path.clone()),
        Ok(Some(reason)) => bump_skip(ctx.out, reason),
        Err(msg) => bump_skip_with_context(ctx.out, SkipReason::Unreadable, msg),
    }
}

pub(super) fn inspect_directory_reparse(ent: &DirEntry) -> Result<Option<SkipReason>, String> {
    let class = sweeper_winfs::open_for_inspection(&ent.path)
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map_err(|e| format!("inspect_reparse_point {}: {e}", ent.path.to_string_lossy()))?;
    Ok(skip_reason_for_reparse(class))
}

pub(super) fn record_file_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    let mtime_secs = super::allowlist::filetime_to_unix_secs(ent.last_write_time);
    let old_enough = is_file_age_eligible(mtime_secs, ctx.now, ctx.min_age);
    *ctx.next_item_seq += 1;
    let item = build_scan_item(ctx, ent, mtime_secs, old_enough);
    accumulate_file_item(ctx.out, &item);
    ctx.progress.scanned_items = ctx.progress.scanned_items.saturating_add(1);
    ctx.progress.scanned_bytes = ctx.progress.scanned_bytes.saturating_add(item.size);
    ctx.out.items.push(item);
}

pub(super) fn is_file_age_eligible(mtime_secs: u64, now: SystemTime, min_age: Duration) -> bool {
    let mtime_st = UNIX_EPOCH + Duration::from_secs(mtime_secs);
    if mtime_st > now {
        return false;
    }
    now.duration_since(mtime_st)
        .map(|d| d >= min_age)
        .unwrap_or(false)
}

pub(super) fn build_scan_item(
    ctx: &ScanWalkCtx<'_>,
    ent: &DirEntry,
    mtime_secs: u64,
    age_eligible: bool,
) -> ScanItem {
    ScanItem {
        id: format!("{}::{}", ctx.target.id, *ctx.next_item_seq),
        target_id: ctx.target.id.clone(),
        path: ent.path.to_string_lossy().to_string(),
        file_identity: file_identity_for_path(&ent.path),
        size: ent.size,
        mtime_unix_secs: mtime_secs,
        age_eligible,
    }
}

pub(super) fn file_identity_for_path(path: &Path) -> Option<FileIdentity> {
    sweeper_winfs::open_for_inspection(path)
        .and_then(|h| sweeper_winfs::get_file_identity(&h))
        .ok()
}

pub(super) fn accumulate_file_item(out: &mut TargetScanResult, item: &ScanItem) {
    out.total_bytes = out.total_bytes.saturating_add(item.size);
    out.file_count = out.file_count.saturating_add(1);
    if item.age_eligible {
        out.eligible_bytes = out.eligible_bytes.saturating_add(item.size);
        out.eligible_count = out.eligible_count.saturating_add(1);
    }
}

pub(super) fn bump_skip(out: &mut TargetScanResult, reason: SkipReason) {
    bump_skip_with_context(out, reason, String::new());
}

pub(super) fn bump_skip_with_context(
    out: &mut TargetScanResult,
    reason: SkipReason,
    context: String,
) {
    out.skipped_count = out.skipped_count.saturating_add(1);
    let entry = out
        .skipped_reasons
        .entry(reason.as_str().to_string())
        .or_insert(0);
    *entry = entry.saturating_add(1);
    if !context.is_empty() {
        out.skipped_diagnostics
            .push(format!("{}: {}", reason.as_str(), context));
    }
}
