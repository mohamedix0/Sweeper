mod markers;
mod postorder;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::owner_lock::{is_owner_lock_held, OWNER_LOCK_FILE_NAME};
use crate::paths::strip_extended_prefix;

pub(crate) use markers::has_canary_markers;
pub(crate) use postorder::remove_dir_all_sandbox_root;

/// Sentinel file name required at the root of every deletion/test sandbox.
pub const SENTINEL_FILE_NAME: &str = ".sweeper-sandbox-sentinel";

/// Directory name prefix for Sweeper-owned test sandboxes.
pub const SANDBOX_DIR_PREFIX: &str = "sweeper-sandbox-";
/// Directory name prefix for Sweeper-owned canary trees.
pub const CANARY_DIR_PREFIX: &str = "sweeper-canary-";

/// Removes stale Sweeper-owned `sweeper-sandbox-*` directories under `base`.
///
/// Returns the number of directories removed.
pub fn cleanup_stale_sandboxes(base: &Path, max_age: Duration) -> io::Result<usize> {
    cleanup_stale_owned_dirs(base, max_age, OwnedTempKind::Sandbox)
}

/// Removes stale Sweeper-owned `sweeper-canary-*` directories under `base`.
pub fn cleanup_stale_canaries(base: &Path, max_age: Duration) -> io::Result<usize> {
    cleanup_stale_owned_dirs(base, max_age, OwnedTempKind::Canary)
}

#[derive(Clone, Copy, Debug)]
enum OwnedTempKind {
    Sandbox,
    Canary,
}

impl OwnedTempKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Sandbox => SANDBOX_DIR_PREFIX,
            Self::Canary => CANARY_DIR_PREFIX,
        }
    }

    fn has_required_marker(self, path: &Path) -> bool {
        match self {
            Self::Sandbox => path.join(SENTINEL_FILE_NAME).is_file(),
            Self::Canary => markers::has_canary_markers(path),
        }
    }
}

fn cleanup_stale_owned_dirs(
    base: &Path,
    max_age: Duration,
    kind: OwnedTempKind,
) -> io::Result<usize> {
    let Some(base_canon) = prepare_cleanup_base(base)? else {
        return Ok(0);
    };
    let ctx = CleanupEntryCtx {
        base_canon: &base_canon,
        now: SystemTime::now(),
        max_age,
        kind,
    };
    let mut removed = 0usize;
    for entry in fs::read_dir(&base_canon)? {
        if try_cleanup_stale_entry(entry?, ctx) {
            removed += 1;
        }
    }
    Ok(removed)
}

fn prepare_cleanup_base(base: &Path) -> io::Result<Option<PathBuf>> {
    let base_canon = canonicalize_for_temp_cleanup(base)?;
    let base_meta = fs::symlink_metadata(&base_canon)?;
    if is_reparse_point(&base_canon, &base_meta) || !base_meta.is_dir() {
        return Ok(None);
    }
    Ok(Some(base_canon))
}

#[derive(Clone, Copy)]
struct CleanupEntryCtx<'a> {
    base_canon: &'a Path,
    now: SystemTime,
    max_age: Duration,
    kind: OwnedTempKind,
}

fn try_cleanup_stale_entry(entry: fs::DirEntry, ctx: CleanupEntryCtx<'_>) -> bool {
    let path = entry.path();
    if !is_stale_cleanup_candidate(&path, ctx) {
        return false;
    }
    if !is_canonical_direct_child(&path, ctx.base_canon) {
        return false;
    }
    postorder::remove_dir_all_sandbox_root(&strip_extended_prefix_canon(&path)).is_ok()
        || postorder::remove_dir_all_sandbox_root(&path).is_ok()
}

fn is_stale_cleanup_candidate(path: &Path, ctx: CleanupEntryCtx<'_>) -> bool {
    if !is_direct_named_child(path, ctx.base_canon, ctx.kind.prefix()) {
        return false;
    }
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || is_reparse_point(path, &meta) {
        return false;
    }
    if !ctx.kind.has_required_marker(path) || !is_older_than(&meta, ctx.now, ctx.max_age) {
        return false;
    }
    !is_owner_lock_held(&path.join(OWNER_LOCK_FILE_NAME))
}

fn is_direct_named_child(path: &Path, base_canon: &Path, prefix: &str) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if !name.starts_with(prefix) {
        return false;
    }
    path.parent()
        .is_some_and(|p| strip_extended_prefix(p.to_path_buf()) == base_canon)
}

fn is_canonical_direct_child(path: &Path, base_canon: &Path) -> bool {
    let child_canon = match fs::canonicalize(path) {
        Ok(p) => strip_extended_prefix(p),
        Err(_) => return false,
    };
    child_canon.parent() == Some(base_canon)
}

fn strip_extended_prefix_canon(path: &Path) -> PathBuf {
    match fs::canonicalize(path) {
        Ok(p) => strip_extended_prefix(p),
        Err(_) => path.to_path_buf(),
    }
}

fn canonicalize_for_temp_cleanup(base: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(base)?;
    let c = fs::canonicalize(base)?;
    Ok(strip_extended_prefix(c))
}

fn is_older_than(meta: &fs::Metadata, now: SystemTime, max_age: Duration) -> bool {
    let mtime = meta.modified().or_else(|_| meta.created()).ok();
    match mtime {
        Some(t) => now.duration_since(t).unwrap_or_default() >= max_age,
        None => false,
    }
}

#[cfg(windows)]
pub(super) fn is_reparse_point(_path: &Path, meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
pub(super) fn is_reparse_point(_path: &Path, meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}
