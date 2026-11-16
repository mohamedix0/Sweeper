use std::fs;
use std::io;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;

use crate::cleanup::{
    cleanup_stale_sandboxes, remove_dir_all_sandbox_root, SANDBOX_DIR_PREFIX, SENTINEL_FILE_NAME,
};
use crate::owner_lock::{acquire_owner_lock, OWNER_LOCK_FILE_NAME};
use crate::paths::{default_sandbox_base_checked, strip_extended_prefix};
use crate::{Result, TestkitError};

/// Per-test isolated sandbox rooted in a temp directory with a required sentinel file.
///
/// Invariant: the sentinel file ([`SENTINEL_FILE_NAME`]) must exist at the sandbox root.
/// Guarded helpers (e.g. [`Sandbox::guarded_path`], [`Sandbox::write_file`]) enforce this
/// before performing filesystem operations.
#[derive(Debug)]
pub struct Sandbox {
    _dir: Option<TempDir>,
    root: PathBuf,
    _owner_lock: Option<fs::File>,
}

impl Sandbox {
    /// Create a new isolated sandbox with a sentinel file at its root.
    pub fn new() -> Self {
        Self::try_new().unwrap_or_else(|e| panic!("{e}"))
    }

    /// Non-panicking sandbox creation with actionable diagnostics when no safe base is available.
    pub fn try_new() -> io::Result<Self> {
        Self::try_new_in(default_sandbox_base_checked()?)
    }

    /// Create a new isolated sandbox under `base_dir` with a sentinel file at its root.
    pub fn new_in(base_dir: impl AsRef<Path>) -> Self {
        Self::try_new_in(base_dir).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Non-panicking sandbox creation in an explicit base directory.
    pub fn try_new_in(base_dir: impl AsRef<Path>) -> io::Result<Self> {
        fs::create_dir_all(base_dir.as_ref())?;
        let _ = cleanup_stale_sandboxes(base_dir.as_ref(), Duration::from_secs(3600));
        let dir = tempfile::Builder::new()
            .prefix(SANDBOX_DIR_PREFIX)
            .tempdir_in(base_dir.as_ref())?;
        let root = dir.path().to_path_buf();
        write_sentinel(&root)?;
        let owner_lock = acquire_owner_lock(&root.join(OWNER_LOCK_FILE_NAME))?;
        Ok(Self {
            _dir: Some(dir),
            root,
            _owner_lock: Some(owner_lock),
        })
    }

    /// Returns the sandbox root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Assert the sentinel exists at the sandbox root. Panics if missing.
    pub fn assert_sentinel_present(&self) {
        let sentinel = self.root.join(SENTINEL_FILE_NAME);
        if !sentinel.is_file() {
            panic!("{}", TestkitError::SentinelMissing(sentinel));
        }
    }

    /// Resolve a path inside the sandbox after verifying the sentinel is present.
    pub fn guarded_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.assert_sentinel_present();
        self.checked_join_relative(relative.as_ref())
            .unwrap_or_else(|e| panic!("{e}"))
    }

    /// Create a child directory inside the sandbox after verifying sentinel.
    pub fn create_dir(&self, relative: impl AsRef<Path>) -> Result<PathBuf> {
        self.assert_sentinel_present();
        let path = self.checked_join_relative(relative.as_ref())?;
        self.assert_path_within_root(&path)?;
        fs::create_dir_all(&path)?;
        self.assert_path_within_root(&path)?;
        Ok(path)
    }

    /// Write a file inside the sandbox after verifying sentinel.
    pub fn write_file(
        &self,
        relative: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> Result<PathBuf> {
        self.assert_sentinel_present();
        let path = self.checked_join_relative(relative.as_ref())?;
        self.assert_path_within_root(&path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            self.assert_path_within_root(parent)?;
        }
        let mut f = fs::File::create(&path)?;
        f.write_all(contents.as_ref())?;
        f.sync_all()?;
        self.assert_path_within_root(&path)?;
        Ok(path)
    }

    fn checked_join_relative(&self, relative: &Path) -> Result<PathBuf> {
        if relative.as_os_str().is_empty() {
            return Err(TestkitError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sandbox relative path must not be empty",
            )));
        }
        for comp in relative.components() {
            match comp {
                Component::Normal(_) | Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(TestkitError::Io(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "sandbox path escapes or is not a safe relative path: {}",
                            relative.display()
                        ),
                    )));
                }
            }
        }
        Ok(self.root.join(relative))
    }

    fn assert_path_within_root(&self, path: &Path) -> Result<()> {
        let root_canon = canonicalize_existing(&self.root)?;
        let existing = deepest_existing_ancestor(path)?;
        let existing_canon = canonicalize_existing(&existing)?;
        if !existing_canon.starts_with(&root_canon) {
            return Err(TestkitError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "sandbox path escapes root (symlink/junction traversal): {}",
                    path.display()
                ),
            )));
        }
        // Lexical containment for the requested path under the sandbox root.
        let root_norm = normalize_lexical(&self.root);
        let path_norm = normalize_lexical(path);
        if !path_norm.starts_with(&root_norm) {
            return Err(TestkitError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("sandbox path escapes root: {}", path.display()),
            )));
        }
        Ok(())
    }
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path)
        .map(strip_extended_prefix)
        .map_err(TestkitError::Io)
}

fn deepest_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut cur = path.to_path_buf();
    loop {
        if cur.exists() || fs::symlink_metadata(&cur).is_ok() {
            return Ok(cur);
        }
        if !cur.pop() {
            return Err(TestkitError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no existing ancestor for path: {}", path.display()),
            )));
        }
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    use std::path::Component as C;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            C::Prefix(_) | C::RootDir => out.push(comp.as_os_str()),
            C::CurDir => {}
            C::ParentDir => {
                out.pop();
            }
            C::Normal(s) => out.push(s),
        }
    }
    out
}

fn write_sentinel(root: &Path) -> io::Result<()> {
    let sentinel = root.join(SENTINEL_FILE_NAME);
    let mut f = fs::File::create(&sentinel)?;
    writeln!(f, "sweeper-testkit sentinel; do not remove during test run")?;
    f.sync_all()?;
    Ok(())
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Disarm TempDir auto-cleanup before sentinel validation so a missing sentinel
        // never allows TempDir's destructor to delete an unowned/non-sandbox directory.
        if let Some(dir) = self._dir.take() {
            let _ = dir.keep();
        }
        if !self.root.join(SENTINEL_FILE_NAME).is_file() {
            // Not a Sweeper-owned sandbox root; preserve contents for inspection/retry safety.
            return;
        }
        // Release the exclusive owner lock before removing lock/marker files.
        let _ = self._owner_lock.take();
        let _ = remove_dir_all_sandbox_root(&self.root);
    }
}
