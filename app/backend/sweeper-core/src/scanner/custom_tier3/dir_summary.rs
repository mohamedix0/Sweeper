use std::path::{Path, PathBuf};

use sweeper_winfs::ReparseTagClass;

use crate::scanner::{allowlist, ScanError, ScanProgressRuntime};

#[derive(Debug, Clone, Copy)]
pub(in crate::scanner) struct DirSummary {
    pub total: u64,
    pub eligible: u64,
    pub count: u64,
    pub eligible_count: u64,
    pub max_mtime: u64,
}

struct SummaryCtx<'a, 'p> {
    stack: Vec<PathBuf>,
    summary: DirSummary,
    progress: &'p mut ScanProgressRuntime,
    target_id: Option<&'a str>,
    target_name: Option<&'a str>,
}

pub(in crate::scanner) fn summarize_directory_for_scan(
    dir: &Path,
    progress: &mut ScanProgressRuntime,
    target_id: Option<&str>,
    target_name: Option<&str>,
) -> Result<DirSummary, ScanError> {
    let mut ctx = SummaryCtx {
        stack: vec![dir.to_path_buf()],
        summary: DirSummary {
            total: 0,
            eligible: 0,
            count: 0,
            eligible_count: 0,
            max_mtime: 0,
        },
        progress,
        target_id,
        target_name,
    };
    while let Some(d) = ctx.stack.pop() {
        scan_summary_dir(&d, &mut ctx)?;
    }
    Ok(ctx.summary)
}

fn scan_summary_dir(dir: &Path, ctx: &mut SummaryCtx<'_, '_>) -> Result<(), ScanError> {
    if ctx.progress.cancelled() {
        return Err(ScanError::Cancelled);
    }
    let entries =
        sweeper_winfs::read_dir_entries(dir).map_err(|e| ScanError::Targets(e.to_string()))?;
    process_summary_entries(entries, ctx)?;
    ctx.progress
        .maybe_emit_throttled(ctx.target_id, ctx.target_name);
    Ok(())
}

fn process_summary_entries(
    entries: Vec<sweeper_winfs::DirEntry>,
    ctx: &mut SummaryCtx<'_, '_>,
) -> Result<(), ScanError> {
    for ent in entries {
        if ctx.progress.cancelled() {
            return Err(ScanError::Cancelled);
        }
        if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
            continue;
        }
        process_summary_entry(ent, ctx);
    }
    Ok(())
}

fn process_summary_entry(ent: sweeper_winfs::DirEntry, ctx: &mut SummaryCtx<'_, '_>) {
    if ent.is_dir {
        ctx.stack.push(ent.path);
        return;
    }
    record_summary_file(ent, ctx);
}

fn record_summary_file(ent: sweeper_winfs::DirEntry, ctx: &mut SummaryCtx<'_, '_>) {
    let mt = allowlist::filetime_to_unix_secs(ent.last_write_time);
    ctx.summary.max_mtime = ctx.summary.max_mtime.max(mt);
    ctx.summary.total = ctx.summary.total.saturating_add(ent.size);
    ctx.summary.eligible = ctx.summary.eligible.saturating_add(ent.size);
    ctx.summary.count = ctx.summary.count.saturating_add(1);
    ctx.summary.eligible_count = ctx.summary.eligible_count.saturating_add(1);
    ctx.progress.scanned_items = ctx.progress.scanned_items.saturating_add(1);
    ctx.progress.scanned_bytes = ctx.progress.scanned_bytes.saturating_add(ent.size);
    ctx.progress
        .maybe_emit_throttled(ctx.target_id, ctx.target_name);
}
