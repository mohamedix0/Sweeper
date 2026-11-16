use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::owner_lock::{is_owner_lock_held, OWNER_LOCK_FILE_NAME};
use crate::paths::{strip_extended_prefix, to_extended_path_for_removal};

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
            Self::Canary => has_canary_markers(path),
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
    let now = SystemTime::now();
    let mut removed = 0usize;
    for entry in fs::read_dir(&base_canon)? {
        if try_cleanup_stale_entry(entry?, &base_canon, now, max_age, kind) {
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

fn try_cleanup_stale_entry(
    entry: fs::DirEntry,
    base_canon: &Path,
    now: SystemTime,
    max_age: Duration,
    kind: OwnedTempKind,
) -> bool {
    let path = entry.path();
    if !is_stale_cleanup_candidate(&path, base_canon, now, max_age, kind) {
        return false;
    }
    if !is_canonical_direct_child(&path, base_canon) {
        return false;
    }
    remove_dir_all_sandbox_root(&strip_extended_prefix_canon(&path)).is_ok()
        || remove_dir_all_sandbox_root(&path).is_ok()
}

fn is_stale_cleanup_candidate(
    path: &Path,
    base_canon: &Path,
    now: SystemTime,
    max_age: Duration,
    kind: OwnedTempKind,
) -> bool {
    if !is_direct_named_child(path, base_canon, kind.prefix()) {
        return false;
    }
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || is_reparse_point(path, &meta) {
        return false;
    }
    if !kind.has_required_marker(path) || !is_older_than(&meta, now, max_age) {
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

pub(crate) fn has_canary_markers(path: &Path) -> bool {
    // Markers seeded by CanaryTree::seed_external: root.txt and nested/{file.txt,data.bin}
    path.join("root.txt").is_file()
        && path.join("nested").join("file.txt").is_file()
        && path.join("nested").join("data.bin").is_file()
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
fn is_reparse_point(_path: &Path, meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_path: &Path, meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn remove_dir_all_sandbox_root(root: &Path) -> io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(root)?;
    validate_removable_directory_root(root, &meta)?;
    let target = to_extended_path_for_removal(root);
    remove_owned_root_no_follow(&target)
}

fn validate_removable_directory_root(root: &Path, meta: &fs::Metadata) -> io::Result<()> {
    if meta.is_dir() && !is_reparse_point(root, meta) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "sandbox root is not a removable directory or is a reparse point",
    ))
}

/// Removes a Sweeper-owned sandbox/canary directory without following reparse points.
#[allow(clippy::disallowed_methods)]
fn remove_owned_root_no_follow(path: &Path) -> io::Result<()> {
    if !validate_owned_root_target(path)? {
        return Ok(());
    }
    let dirs_postorder = collect_dirs_postorder(path)?;
    remove_dirs_postorder(path, dirs_postorder)
}

fn validate_owned_root_target(path: &Path) -> io::Result<bool> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    validate_removable_directory_root(path, &meta)?;
    Ok(true)
}

fn collect_dirs_postorder(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut stack = vec![path.to_path_buf()];
    let mut dirs_postorder = Vec::new();
    while let Some(dir) = stack.pop() {
        if !is_traversable_owned_dir(&dir) {
            continue;
        }
        dirs_postorder.push(dir.clone());
        collect_dir_children(path, &dir, &mut stack)?;
    }
    Ok(dirs_postorder)
}

fn is_traversable_owned_dir(dir: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return false;
    };
    meta.is_dir() && !is_reparse_point(dir, &meta) && !meta.file_type().is_symlink()
}

fn collect_dir_children(root: &Path, dir: &Path, stack: &mut Vec<PathBuf>) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        process_removal_child(root, dir, &entry?.path(), stack)?;
    }
    Ok(())
}

fn process_removal_child(
    root: &Path,
    dir: &Path,
    child: &Path,
    stack: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if is_deferred_ownership_marker(root, dir, child) {
        return Ok(());
    }
    let meta = match fs::symlink_metadata(child) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if is_directory_reparse_entry(child, &meta) {
        return remove_reparse_entry(child, &meta);
    }
    if is_plain_directory_child(child, &meta) {
        stack.push(child.to_path_buf());
        return Ok(());
    }
    remove_file_if_exists(child)
}

fn is_plain_directory_child(child: &Path, meta: &fs::Metadata) -> bool {
    meta.is_dir() && !is_reparse_point(child, meta)
}

fn remove_dirs_postorder(path: &Path, dirs_postorder: Vec<PathBuf>) -> io::Result<()> {
    for dir in dirs_postorder.into_iter().rev() {
        remove_single_postorder_dir(path, &dir)?;
    }
    Ok(())
}

#[allow(clippy::disallowed_methods)]
fn remove_single_postorder_dir(root: &Path, dir: &Path) -> io::Result<()> {
    match fs::remove_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) if dir == root && is_retryable_dir_not_empty(&e) => remove_root_after_markers(root),
        Err(e) => Err(e),
    }
}

#[allow(clippy::disallowed_methods)]
fn remove_root_after_markers(root: &Path) -> io::Result<()> {
    remove_ownership_markers(root)?;
    match fs::remove_dir(root) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn is_retryable_dir_not_empty(e: &io::Error) -> bool {
    // Windows ERROR_DIR_NOT_EMPTY=145, POSIX ENOTEMPTY varies; fall back to string match.
    matches!(e.raw_os_error(), Some(145 | 39))
        || e.kind()
            .to_string()
            .to_ascii_lowercase()
            .contains("not empty")
}

fn is_deferred_ownership_marker(root: &Path, current_dir: &Path, child: &Path) -> bool {
    let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name == OWNER_LOCK_FILE_NAME {
        return true;
    }
    if root.join(SENTINEL_FILE_NAME).is_file() {
        return current_dir == root && name == SENTINEL_FILE_NAME;
    }
    is_deferred_canary_marker(root, current_dir, name)
}

fn is_deferred_canary_marker(root: &Path, current_dir: &Path, name: &str) -> bool {
    if !has_canary_markers(root) {
        return false;
    }
    if current_dir == root && name == "root.txt" {
        return true;
    }
    current_dir == root && name == "nested"
}

#[allow(clippy::disallowed_methods)]
fn remove_ownership_markers(root: &Path) -> io::Result<()> {
    remove_file_if_exists(&root.join(OWNER_LOCK_FILE_NAME))?;
    remove_sandbox_sentinel_marker(root)?;
    remove_canary_markers_if_present(root)
}

fn remove_sandbox_sentinel_marker(root: &Path) -> io::Result<()> {
    let sentinel = root.join(SENTINEL_FILE_NAME);
    if !sentinel.exists() && fs::symlink_metadata(&sentinel).is_err() {
        return Ok(());
    }
    remove_file_if_exists(&sentinel)
}

fn remove_canary_markers_if_present(root: &Path) -> io::Result<()> {
    if !has_canary_markers(root) && !root.join("root.txt").exists() {
        return Ok(());
    }
    for p in canary_marker_paths(root) {
        remove_marker_path(&p)?;
    }
    Ok(())
}

fn canary_marker_paths(root: &Path) -> [PathBuf; 4] {
    [
        root.join("nested").join("data.bin"),
        root.join("nested").join("file.txt"),
        root.join("nested"),
        root.join("root.txt"),
    ]
}

fn remove_marker_path(path: &Path) -> io::Result<()> {
    let Ok(m) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if m.is_dir() && !is_reparse_point(path, &m) {
        return remove_dir_if_exists(path);
    }
    remove_file_if_exists(path)
}

#[allow(clippy::disallowed_methods)]
fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[allow(clippy::disallowed_methods)]
fn remove_dir_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
fn is_directory_reparse_entry(_path: &Path, meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::{FileTypeExt, MetadataExt};
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    let attrs = meta.file_attributes();
    if (attrs & FILE_ATTRIBUTE_REPARSE_POINT) == 0 {
        return false;
    }
    (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0 || meta.file_type().is_symlink_dir()
}

#[cfg(not(windows))]
fn is_directory_reparse_entry(_path: &Path, meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[allow(clippy::disallowed_methods)]
fn remove_reparse_entry(path: &Path, meta: &fs::Metadata) -> io::Result<()> {
    // Directory junctions/symlink-dirs must be unlinked with remove_dir; files with remove_file.
    let res = if is_directory_reparse_entry(path, meta) || meta.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    match res {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
