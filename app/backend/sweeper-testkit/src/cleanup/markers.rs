use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::owner_lock::OWNER_LOCK_FILE_NAME;

use super::SENTINEL_FILE_NAME;

pub(crate) fn has_canary_markers(path: &Path) -> bool {
    // Markers seeded by CanaryTree::seed_external: root.txt and nested/{file.txt,data.bin}
    if !path.join("root.txt").is_file() {
        return false;
    }
    if !path.join("nested").join("file.txt").is_file() {
        return false;
    }
    path.join("nested").join("data.bin").is_file()
}

pub(super) fn is_deferred_ownership_marker(root: &Path, current_dir: &Path, child: &Path) -> bool {
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

pub(super) fn is_deferred_canary_marker(root: &Path, current_dir: &Path, name: &str) -> bool {
    if !has_canary_markers(root) {
        return false;
    }
    if current_dir == root && name == "root.txt" {
        return true;
    }
    is_canary_nested_marker_file(root, current_dir, name)
}

pub(super) fn is_canary_nested_marker_file(root: &Path, current_dir: &Path, name: &str) -> bool {
    current_dir == root.join("nested") && matches!(name, "file.txt" | "data.bin")
}

#[allow(clippy::disallowed_methods)]
pub(super) fn remove_ownership_markers(root: &Path) -> io::Result<()> {
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
        root.join("nested").join("file.txt"),
        root.join("nested").join("data.bin"),
        root.join("nested"),
        root.join("root.txt"),
    ]
}

fn remove_marker_path(path: &Path) -> io::Result<()> {
    let Ok(m) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if m.is_dir() && !super::is_reparse_point(path, &m) {
        return remove_dir_if_exists(path);
    }
    remove_file_if_exists(path)
}

#[allow(clippy::disallowed_methods)]
pub(super) fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[allow(clippy::disallowed_methods)]
pub(super) fn remove_dir_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(windows)]
pub(super) fn is_directory_reparse_entry(_path: &Path, meta: &fs::Metadata) -> bool {
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
pub(super) fn is_directory_reparse_entry(_path: &Path, meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

#[allow(clippy::disallowed_methods)]
pub(super) fn remove_reparse_entry(path: &Path, meta: &fs::Metadata) -> io::Result<()> {
    // Directory junctions/symlink-dirs must be unlinked with remove_dir; file symlinks/files with remove_file.
    let res = if should_remove_reparse_as_dir(path, meta) {
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

#[cfg(windows)]
fn should_remove_reparse_as_dir(path: &Path, meta: &fs::Metadata) -> bool {
    is_directory_reparse_entry(path, meta) || meta.is_dir()
}

#[cfg(not(windows))]
fn should_remove_reparse_as_dir(_path: &Path, meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink() && meta.is_dir()
}
