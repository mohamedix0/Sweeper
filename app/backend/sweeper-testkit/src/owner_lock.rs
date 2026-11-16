use std::fs;
use std::io;
use std::path::Path;

/// Owner lock file held for the lifetime of a Sweeper-owned sandbox/canary instance.
pub const OWNER_LOCK_FILE_NAME: &str = ".sweeper-owner.lock";

pub(crate) fn acquire_owner_lock(path: &Path) -> io::Result<fs::File> {
    use std::fs::OpenOptions;
    let mut opts = OpenOptions::new();
    opts.create(true).read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Exclusive lock for liveness detection: any concurrent open/cleanup attempt fails while held.
        opts.share_mode(0);
    }
    let file = opts.open(path)?;
    // Best-effort: ensure file exists and is lock marker.
    let _ = file.set_len(0);
    Ok(file)
}

pub(crate) fn is_owner_lock_held(lock_path: &Path) -> bool {
    if !lock_path.exists() {
        return false;
    }
    #[cfg(windows)]
    {
        is_owner_lock_held_windows(lock_path)
    }
    #[cfg(not(windows))]
    {
        let _ = lock_path;
        false
    }
}

#[cfg(windows)]
fn is_owner_lock_held_windows(lock_path: &Path) -> bool {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).share_mode(0);
    match opts.open(lock_path) {
        Ok(_f) => {
            // Successfully acquired exclusive access => lock not held by a live owner.
            false
        }
        Err(e) => {
            matches!(e.raw_os_error(), Some(32 | 33 | 5))
                || e.kind() == io::ErrorKind::PermissionDenied
        }
    }
}
