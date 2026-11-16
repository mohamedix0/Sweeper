use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::{Result, TestkitError};

/// Create a directory junction at `link` pointing to `target` (Windows).
///
/// Uses a direct Win32 reparse-point implementation (no `cmd.exe` spawn) for speed/reliability.
#[cfg(windows)]
pub fn create_junction(link: &Path, target: &Path) -> Result<()> {
    sweeper_winfs::create_directory_junction(link, target)
        .map_err(|e| TestkitError::Io(std::io::Error::other(e.to_string())))
}

/// Create a directory junction at `link` pointing to `target` (non-Windows stub).
#[cfg(not(windows))]
pub fn create_junction(link: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

/// Create a file symlink at `link` pointing to `target` (Windows).
#[cfg(windows)]
pub fn create_file_symlink(link: &Path, target: &Path) -> Result<()> {
    match std::os::windows::fs::symlink_file(target, link) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(1314) => Err(TestkitError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "symlink privilege not held (enable Developer Mode or run with required privilege): {e}"
            ),
        ))),
        Err(e) => Err(TestkitError::Io(e)),
    }
}

/// Create a file symlink at `link` pointing to `target` (non-Windows stub).
#[cfg(not(windows))]
pub fn create_file_symlink(link: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}

/// Create a hard link at `link` pointing to existing file `target`.
pub fn create_hard_link(link: &Path, target: &Path) -> Result<()> {
    fs::hard_link(target, link)?;
    Ok(())
}

/// Write bytes to an NTFS alternate data stream (e.g. `base_path` + `:stream:$DATA`).
#[cfg(windows)]
pub fn write_ads(base_path: &Path, stream_suffix: &str, contents: &[u8]) -> Result<PathBuf> {
    let ads = PathBuf::from(format!("{}{}", base_path.to_string_lossy(), stream_suffix));
    let mut f = fs::File::create(&ads)?;
    f.write_all(contents)?;
    f.sync_all()?;
    Ok(ads)
}

/// Read bytes from an NTFS alternate data stream.
#[cfg(windows)]
pub fn read_ads(base_path: &Path, stream_suffix: &str) -> Result<Vec<u8>> {
    let ads = PathBuf::from(format!("{}{}", base_path.to_string_lossy(), stream_suffix));
    let mut f = fs::File::open(&ads)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Set file attributes via `attrib` (for test setup), e.g. read-only/hidden/system.
///
/// `attrs` uses Win32 attribute bits: READONLY(0x1), HIDDEN(0x2), SYSTEM(0x4).
#[cfg(windows)]
pub fn set_file_attributes(path: &Path, attrs: u32) -> Result<()> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    clear_file_attributes(path, CREATE_NO_WINDOW);
    apply_file_attributes(path, attrs, CREATE_NO_WINDOW)
}

#[cfg(windows)]
fn clear_file_attributes(path: &Path, creation_flags: u32) {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("attrib");
    cmd.creation_flags(creation_flags);
    cmd.arg("-R").arg("-H").arg("-S").arg(path);
    let _ = cmd.status();
}

#[cfg(windows)]
fn apply_file_attributes(path: &Path, attrs: u32, creation_flags: u32) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let mut cmd = std::process::Command::new("attrib");
    cmd.creation_flags(creation_flags);
    if (attrs & FILE_ATTRIBUTE_READONLY) != 0 {
        cmd.arg("+R");
    }
    if (attrs & FILE_ATTRIBUTE_HIDDEN) != 0 {
        cmd.arg("+H");
    }
    if (attrs & FILE_ATTRIBUTE_SYSTEM) != 0 {
        cmd.arg("+S");
    }
    cmd.arg(path);
    let status = cmd.status()?;
    if status.success() {
        return Ok(());
    }
    Err(TestkitError::Io(std::io::Error::other(format!(
        "attrib failed for {}",
        path.display()
    ))))
}

/// Set last write time for a path (FILETIME as 100ns intervals since 1601) for test setup.
#[cfg(windows)]
pub fn set_last_write_time(path: &Path, filetime: u64) -> Result<()> {
    sweeper_winfs::set_last_write_time(path, filetime)
        .map_err(|e| TestkitError::Io(std::io::Error::other(e.to_string())))
}

/// Create a deep directory tree (iterative) of `depth` levels under `root`, returning the deepest path.
pub fn create_deep_tree(root: &Path, depth: usize) -> Result<PathBuf> {
    let mut cur = root.to_path_buf();
    fs::create_dir_all(&cur)?;
    for i in 0..depth {
        cur.push(format!("d{i:04}"));
        fs::create_dir(&cur)?;
    }
    Ok(cur)
}

/// Create a file in a directory, returning its path.
pub fn create_file(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(contents)?;
    f.sync_all()?;
    Ok(())
}

/// Build a `\\?\` extended path for an absolute Windows path.
#[cfg(windows)]
pub fn to_extended_length_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") {
        path.to_path_buf()
    } else if let Some(rest) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{}", rest))
    } else {
        PathBuf::from(format!(r"\\?\{}", s))
    }
}

/// Helper: current FILETIME for "now".
#[cfg(windows)]
pub fn filetime_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    dur.as_secs() * 10_000_000 + u64::from(dur.subsec_nanos() / 100) + EPOCH_DIFF_100NS
}

/// Helper: FILETIME from unix seconds.
pub fn filetime_from_unix_secs(secs: u64) -> u64 {
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    secs.saturating_mul(10_000_000)
        .saturating_add(EPOCH_DIFF_100NS)
}
