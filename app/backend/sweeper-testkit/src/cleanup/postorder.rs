use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::paths::to_extended_path_for_removal;

use super::{is_reparse_point, markers};

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
    if !meta.is_dir() {
        return false;
    }
    if is_reparse_point(dir, &meta) {
        return false;
    }
    !meta.file_type().is_symlink()
}

struct RemovalWalkCtx<'a> {
    root: &'a Path,
    dir: &'a Path,
    stack: &'a mut Vec<PathBuf>,
}

fn collect_dir_children(root: &Path, dir: &Path, stack: &mut Vec<PathBuf>) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut ctx = RemovalWalkCtx { root, dir, stack };
    for entry in entries {
        process_removal_child(&mut ctx, &entry?.path())?;
    }
    Ok(())
}

fn process_removal_child(ctx: &mut RemovalWalkCtx<'_>, child: &Path) -> io::Result<()> {
    if markers::is_deferred_ownership_marker(ctx.root, ctx.dir, child) {
        return Ok(());
    }
    let meta = match fs::symlink_metadata(child) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if markers::is_directory_reparse_entry(child, &meta) {
        return markers::remove_reparse_entry(child, &meta);
    }
    if is_plain_directory_child(child, &meta) {
        ctx.stack.push(child.to_path_buf());
        return Ok(());
    }
    markers::remove_file_if_exists(child)
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
        Err(e) => handle_postorder_remove_error(root, dir, e),
    }
}

fn handle_postorder_remove_error(root: &Path, dir: &Path, e: io::Error) -> io::Result<()> {
    if is_retryable_root_dir_not_empty(root, dir, &e) {
        return remove_root_after_markers(root);
    }
    if is_deferred_nested_dir_not_empty(root, dir, &e) {
        return Ok(());
    }
    Err(e)
}

fn is_retryable_root_dir_not_empty(root: &Path, dir: &Path, e: &io::Error) -> bool {
    dir == root && is_retryable_dir_not_empty(e)
}

fn is_deferred_nested_dir_not_empty(root: &Path, dir: &Path, e: &io::Error) -> bool {
    is_deferred_canary_nested_dir(root, dir) && is_retryable_dir_not_empty(e)
}

fn is_deferred_canary_nested_dir(root: &Path, dir: &Path) -> bool {
    markers::has_canary_markers(root) && dir == root.join("nested")
}

#[allow(clippy::disallowed_methods)]
fn remove_root_after_markers(root: &Path) -> io::Result<()> {
    markers::remove_ownership_markers(root)?;
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
