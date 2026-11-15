//! Test harness: isolated sandbox with sentinel, canary harness, junction/symlink helpers.
//!
//! Phase 0: provides NTFS-safe test primitives without any deletion APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;
use thiserror::Error;
use walkdir::WalkDir;

/// Sentinel file name required at the root of every deletion/test sandbox.
pub const SENTINEL_FILE_NAME: &str = ".sweeper-sandbox-sentinel";

/// Owner lock file held for the lifetime of a Sweeper-owned sandbox/canary instance.
pub const OWNER_LOCK_FILE_NAME: &str = ".sweeper-owner.lock";

/// Directory name prefix for Sweeper-owned test sandboxes.
pub const SANDBOX_DIR_PREFIX: &str = "sweeper-sandbox-";
/// Directory name prefix for Sweeper-owned canary trees.
pub const CANARY_DIR_PREFIX: &str = "sweeper-canary-";

/// Errors returned by the testkit.
#[derive(Debug, Error)]
pub enum TestkitError {
    /// I/O error while creating or inspecting sandbox/canary state.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Walkdir traversal error.
    #[error("walk error: {0}")]
    Walk(String),
    /// Sandbox sentinel is missing.
    #[error("sandbox sentinel missing at {0}")]
    SentinelMissing(PathBuf),
    /// Canary hash verification failed.
    #[error("canary hash mismatch for {path}: expected {expected}, found {found}")]
    CanaryMismatch {
        /// Relative path within the canary tree.
        path: String,
        /// Expected SHA-256 hex digest.
        expected: String,
        /// Found SHA-256 hex digest.
        found: String,
    },
    /// Canary file missing compared to manifest.
    #[error("canary file missing: {0}")]
    CanaryMissing(String),
    /// Unexpected extra file in canary tree.
    #[error("canary unexpected file: {0}")]
    CanaryUnexpected(String),
}

/// Result type for testkit operations.
pub type Result<T> = std::result::Result<T, TestkitError>;

/// Per-test isolated sandbox rooted in a temp directory with a required sentinel file.
///
/// Invariant: the sentinel file ([`SENTINEL_FILE_NAME`]) must exist at the sandbox root.
/// Guarded helpers (e.g. [`Sandbox::guarded_path`], [`Sandbox::write_file`]) enforce this
/// before performing filesystem operations. Any test harness operation that could be used to
/// stage deletion targets must refuse if the sentinel is missing.
#[derive(Debug)]
pub struct Sandbox {
    _dir: Option<TempDir>,
    root: PathBuf,
    _owner_lock: Option<fs::File>,
}

impl Sandbox {
    /// Create a new isolated sandbox with a sentinel file at its root.
    ///
    /// Prefers a canonicalized process temp directory so that Windows short-name aliases in
    /// environment paths (for example `RUNNER~1` on GitHub Actions) are expanded to long names
    /// while preserving temp-area semantics required by allowlist-sensitive tests (e.g. Gate F).
    /// Falls back to a cargo-target sandbox base only when canonicalized temp is unavailable or
    /// still contains short-name segments.
    ///
    /// Opportunistically cleans up stale, sentinel-marked `sweeper-sandbox-*` directories left in
    /// the selected base by previously aborted test runs (age-gated, never touching active runs).
    pub fn new() -> Self {
        Self::try_new().unwrap_or_else(|e| panic!("{e}"))
    }

    /// Non-panicking sandbox creation with actionable diagnostics when no safe base is available.
    pub fn try_new() -> std::io::Result<Self> {
        Self::try_new_in(default_sandbox_base_checked()?)
    }

    /// Create a new isolated sandbox under `base_dir` with a sentinel file at its root.
    ///
    /// Useful for tests that must satisfy path-policy constraints that reject locations
    /// under protected container roots (e.g. `%TEMP%` inside a user profile on Windows).
    ///
    /// Before creating a new sandbox, attempts opportunistic cleanup of stale `sweeper-sandbox-*`
    /// orphans in `base_dir` (best-effort; failures are ignored so sandbox creation can proceed).
    pub fn new_in(base_dir: impl AsRef<Path>) -> Self {
        Self::try_new_in(base_dir).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Non-panicking sandbox creation in an explicit base directory.
    pub fn try_new_in(base_dir: impl AsRef<Path>) -> std::io::Result<Self> {
        fs::create_dir_all(base_dir.as_ref())?;
        let _ = cleanup_stale_sandboxes(base_dir.as_ref(), Duration::from_secs(3600));
        let dir = tempfile::Builder::new()
            .prefix(SANDBOX_DIR_PREFIX)
            .tempdir_in(base_dir.as_ref())?;
        let root = dir.path().to_path_buf();
        let sentinel = root.join(SENTINEL_FILE_NAME);
        let mut f = fs::File::create(&sentinel)?;
        writeln!(f, "sweeper-testkit sentinel; do not remove during test run")?;
        f.sync_all()?;
        let owner_lock = acquire_owner_lock(&root.join(OWNER_LOCK_FILE_NAME))?;
        Ok(Self {
            _dir: Some(dir),
            root,
            _owner_lock: Some(owner_lock),
        })
    }

    /// Returns the sandbox root path.
    ///
    /// Callers must treat this as a sentinel-guarded root: use [`Sandbox::guarded_path`] or
    /// other guarded helpers before filesystem mutations, and never perform deletion-like test
    /// operations if [`Sandbox::assert_sentinel_present`] would fail.
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
        self.root.join(relative)
    }

    /// Create a child directory inside the sandbox after verifying sentinel.
    pub fn create_dir(&self, relative: impl AsRef<Path>) -> Result<PathBuf> {
        let path = self.guarded_path(relative);
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    /// Write a file inside the sandbox after verifying sentinel.
    pub fn write_file(
        &self,
        relative: impl AsRef<Path>,
        contents: impl AsRef<[u8]>,
    ) -> Result<PathBuf> {
        let path = self.guarded_path(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::File::create(&path)?;
        f.write_all(contents.as_ref())?;
        f.sync_all()?;
        Ok(path)
    }
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
        let dir = self._dir.take();
        if let Some(dir) = dir {
            let _ = dir.keep();
        }

        let sentinel = self.root.join(SENTINEL_FILE_NAME);
        if !sentinel.is_file() {
            // Not a Sweeper-owned sandbox root; preserve contents for inspection/retry safety.
            return;
        }

        // Release the exclusive owner lock before removing lock/marker files.
        let _ = self._owner_lock.take();
        let _ = remove_dir_all_sandbox_root(&self.root);
    }
}

/// Removes stale Sweeper-owned `sweeper-sandbox-*` directories under `base`.
///
/// Safety guards (all required before removal):
/// 1. `base` is treated as the canonical temp/base directory; each candidate must be a direct child.
/// 2. Directory name must start with [`SANDBOX_DIR_PREFIX`].
/// 3. Candidate must not be a reparse point / symlink / junction.
/// 4. Candidate must contain [`SENTINEL_FILE_NAME`].
/// 5. Candidate must be older than `max_age` (based on filesystem modified time) so active
///    parallel test runs are never disturbed.
///
/// Returns the number of directories removed.
pub fn cleanup_stale_sandboxes(base: &Path, max_age: Duration) -> std::io::Result<usize> {
    cleanup_stale_owned_dirs(base, max_age, OwnedTempKind::Sandbox)
}

/// Removes stale Sweeper-owned `sweeper-canary-*` directories under `base`.
///
/// Safety guards mirror sandbox cleanup except the marker check: a canary must contain at least
/// one of the deterministic canary files seeded by [`CanaryTree::seed_external`].
pub fn cleanup_stale_canaries(base: &Path, max_age: Duration) -> std::io::Result<usize> {
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
) -> std::io::Result<usize> {
    let base_canon = canonicalize_for_temp_cleanup(base)?;
    // Never operate on a reparse-point base.
    let base_meta = fs::symlink_metadata(&base_canon)?;
    if is_reparse_point(&base_canon, &base_meta) || !base_meta.is_dir() {
        return Ok(0);
    }

    let now = SystemTime::now();
    let mut removed = 0usize;

    for entry in fs::read_dir(&base_canon)? {
        let entry = entry?;
        let path = entry.path();

        // Direct child containment check without following reparse points on the child.
        let Some(name) = path.file_name() else {
            continue;
        };
        let parent_ok = path
            .parent()
            .is_some_and(|p| strip_extended_prefix(p.to_path_buf()) == base_canon);
        if !parent_ok {
            // read_dir yielded unexpected path; require direct child relationship.
            continue;
        }
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !name_str.starts_with(kind.prefix()) {
            continue;
        }

        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Must not be a reparse point / symlink / junction.
        if !meta.is_dir() || is_reparse_point(&path, &meta) {
            continue;
        }
        if !kind.has_required_marker(&path) {
            continue;
        }
        if !is_older_than(&meta, now, max_age) {
            continue;
        }
        // Never remove directories owned by a live Sandbox/CanaryTree instance.
        if is_owner_lock_held(&path.join(OWNER_LOCK_FILE_NAME)) {
            continue;
        }

        // Final guard: canonicalized child must still be a direct child of canonical base.
        let child_canon = match fs::canonicalize(&path) {
            Ok(p) => strip_extended_prefix(p),
            Err(_) => continue,
        };
        if child_canon.parent() != Some(base_canon.as_path()) {
            continue;
        }

        if remove_dir_all_sandbox_root(&child_canon).is_ok()
            || remove_dir_all_sandbox_root(&path).is_ok()
        {
            removed += 1;
        }
    }

    Ok(removed)
}

fn has_canary_markers(path: &Path) -> bool {
    // Markers seeded by CanaryTree::seed_external: root.txt and nested/{file.txt,data.bin}
    path.join("root.txt").is_file()
        && path.join("nested").join("file.txt").is_file()
        && path.join("nested").join("data.bin").is_file()
}

fn canonicalize_for_temp_cleanup(base: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(base)?;
    let c = fs::canonicalize(base)?;
    Ok(strip_extended_prefix(c))
}

// (helper removed; direct comparisons use strip_extended_prefix)

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
fn remove_dir_all_sandbox_root(root: &Path) -> std::io::Result<()> {
    if !root.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() || is_reparse_point(root, &meta) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sandbox root is not a removable directory or is a reparse point",
        ));
    }
    let target = to_extended_path_for_removal(root);
    remove_owned_root_no_follow(&target)
}

#[cfg(windows)]
fn to_extended_path_for_removal(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") {
        path.to_path_buf()
    } else if let Some(rest) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{}", rest))
    } else {
        PathBuf::from(format!(r"\\?\{}", s))
    }
}

#[cfg(not(windows))]
fn to_extended_path_for_removal(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Removes a Sweeper-owned sandbox/canary directory without following reparse points.
///
/// Ownership markers are removed only after all other contents are removed successfully so a
/// partial failure leaves the directory recognizable for safe retry/cleanup.
#[allow(clippy::disallowed_methods)]
fn remove_owned_root_no_follow(path: &Path) -> std::io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let ft = meta.file_type();
    if ft.is_symlink() || is_reparse_point(path, &meta) || !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "owned root is not a removable directory or is a reparse point",
        ));
    }

    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    let mut dirs_postorder: Vec<PathBuf> = Vec::new();

    while let Some(dir) = stack.pop() {
        let dir_meta = match fs::symlink_metadata(&dir) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if dir_meta.file_type().is_symlink()
            || is_reparse_point(&dir, &dir_meta)
            || !dir_meta.is_dir()
        {
            continue;
        }

        dirs_postorder.push(dir.clone());
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };

        for entry in entries {
            let entry = entry?;
            let child = entry.path();
            if is_deferred_ownership_marker(path, &dir, &child) {
                continue;
            }

            let child_meta = match fs::symlink_metadata(&child) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };

            if is_directory_reparse_entry(&child, &child_meta) {
                remove_reparse_entry(&child, &child_meta)?;
                continue;
            }

            if child_meta.is_dir() && !is_reparse_point(&child, &child_meta) {
                stack.push(child);
            } else {
                match fs::remove_file(&child) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }

    for dir in dirs_postorder.into_iter().rev() {
        match fs::remove_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                // Root dir may still contain ownership markers; remove them only after
                // all other contents/dirs were removed successfully.
                if dir == path && is_retryable_dir_not_empty(&e) {
                    remove_ownership_markers(path)?;
                    match fs::remove_dir(&dir) {
                        Ok(()) => {}
                        Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e2) => return Err(e2),
                    }
                    continue;
                }
                return Err(e);
            }
        }
    }

    Ok(())
}

fn is_retryable_dir_not_empty(e: &std::io::Error) -> bool {
    // Windows ERROR_DIR_NOT_EMPTY=145, POSIX ENOTEMPTY varies; fall back to string match.
    matches!(e.raw_os_error(), Some(145 | 39))
        || e.kind()
            .to_string()
            .to_ascii_lowercase()
            .contains("not empty")
}

fn is_deferred_ownership_marker(root: &Path, current_dir: &Path, child: &Path) -> bool {
    let name = child.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name == OWNER_LOCK_FILE_NAME {
        return true;
    }
    let root_is_sandbox = root.join(SENTINEL_FILE_NAME).is_file();
    if root_is_sandbox {
        return current_dir == root && name == SENTINEL_FILE_NAME;
    }
    // Canary roots: preserve canary identity markers until all other contents are removed.
    let root_is_canary = has_canary_markers(root);
    if root_is_canary {
        if current_dir == root && name == "root.txt" {
            return true;
        }
        if current_dir == root && name == "nested" {
            // defer entire nested marker dir (contains marker files) until final cleanup
            return true;
        }
    }
    false
}

#[allow(clippy::disallowed_methods)]
fn remove_ownership_markers(root: &Path) -> std::io::Result<()> {
    // Remove owner lock marker first (handle already dropped by caller), then sandbox/canary markers.
    let lock = root.join(OWNER_LOCK_FILE_NAME);
    match fs::remove_file(&lock) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // Sandbox sentinel
    let sentinel = root.join(SENTINEL_FILE_NAME);
    if sentinel.exists() || fs::symlink_metadata(&sentinel).is_ok() {
        match fs::remove_file(&sentinel) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    // Canary markers (remove only after non-marker contents are gone)
    if has_canary_markers(root) || root.join("root.txt").exists() {
        for p in [
            root.join("nested").join("data.bin"),
            root.join("nested").join("file.txt"),
            root.join("nested"),
            root.join("root.txt"),
        ] {
            if let Ok(m) = fs::symlink_metadata(&p) {
                let res = if m.is_dir() && !is_reparse_point(&p, &m) {
                    fs::remove_dir(&p)
                } else {
                    fs::remove_file(&p)
                };
                match res {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok(())
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
fn remove_reparse_entry(path: &Path, meta: &fs::Metadata) -> std::io::Result<()> {
    // Directory junctions/symlink-dirs must be unlinked with remove_dir; files with remove_file.
    let res = if is_directory_reparse_entry(path, meta) || meta.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    match res {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// External canary tree with SHA-256 manifest for post-run integrity verification.
#[derive(Debug)]
pub struct CanaryTree {
    _dir: Option<TempDir>,
    root: PathBuf,
    manifest: BTreeMap<String, String>,
    _owner_lock: Option<fs::File>,
}

impl CanaryTree {
    /// Seed an external canary tree (outside any test sandbox) with deterministic files.
    pub fn seed_external(label: &str) -> Result<Self> {
        // Best-effort cleanup of stale canaries in the OS temp dir (age-gated, safe guards).
        let _ = cleanup_stale_canaries(&std::env::temp_dir(), Duration::from_secs(3600));
        let dir = tempfile::Builder::new()
            .prefix(&format!("{CANARY_DIR_PREFIX}{label}-"))
            .tempdir()?;
        let root = dir.path().to_path_buf();

        // Seed a small deterministic tree.
        fs::create_dir_all(root.join("nested"))?;
        write_bytes(root.join("root.txt"), b"canary-root")?;
        write_bytes(root.join("nested").join("file.txt"), b"canary-nested")?;
        write_bytes(root.join("nested").join("data.bin"), &[0, 1, 2, 3, 4, 5])?;
        let owner_lock = acquire_owner_lock(&root.join(OWNER_LOCK_FILE_NAME))?;

        let manifest = build_manifest(&root)?;
        Ok(Self {
            _dir: Some(dir),
            root,
            manifest,
            _owner_lock: Some(owner_lock),
        })
    }

    /// Returns the canary root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Verify all canary files still exist with identical SHA-256 hashes.
    pub fn verify(&self) -> Result<()> {
        let current = build_manifest(&self.root)?;
        for (rel, expected) in &self.manifest {
            match current.get(rel) {
                None => return Err(TestkitError::CanaryMissing(rel.clone())),
                Some(found) if found != expected => {
                    return Err(TestkitError::CanaryMismatch {
                        path: rel.clone(),
                        expected: expected.clone(),
                        found: found.clone(),
                    })
                }
                Some(_) => {}
            }
        }
        for rel in current.keys() {
            if !self.manifest.contains_key(rel) {
                return Err(TestkitError::CanaryUnexpected(rel.clone()));
            }
        }
        Ok(())
    }
}

impl Drop for CanaryTree {
    fn drop(&mut self) {
        // Disarm TempDir auto-cleanup so marker/lock-aware cleanup controls removal.
        if let Some(dir) = self._dir.take() {
            let _ = dir.keep();
        }
        if !has_canary_markers(&self.root) {
            return;
        }
        let _ = self._owner_lock.take();
        let _ = remove_dir_all_sandbox_root(&self.root);
    }
}

fn write_bytes(path: PathBuf, bytes: &[u8]) -> Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

fn build_manifest(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut manifest = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|e| TestkitError::Walk(e.to_string()))?;
        if entry.file_type().is_file() {
            if entry.file_name() == OWNER_LOCK_FILE_NAME {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(root)
                .map_err(|e| TestkitError::Walk(e.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let hash = sha256_file(entry.path())?;
            manifest.insert(rel, hash);
        }
    }
    Ok(manifest)
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Selects the default base directory for test sandboxes.
///
/// Resolution order:
/// 1. `SWEEPER_SANDBOX_BASE` (explicit, documented override), if set and safe
/// 2. Canonicalized process temp directory, if safe
/// 3. Cargo target sandbox locations (`CARGO_TARGET_TMPDIR`/`CARGO_TARGET_DIR`/workspace target), if safe
pub fn default_sandbox_base() -> PathBuf {
    default_sandbox_base_checked().unwrap_or_else(|e| panic!("{e}"))
}

/// Non-panicking sandbox base resolution with actionable diagnostics.
pub fn default_sandbox_base_checked() -> std::io::Result<PathBuf> {
    if let Some(v) = std::env::var_os("SWEEPER_SANDBOX_BASE") {
        let raw = PathBuf::from(&v);
        let normalized = normalize_sandbox_base_path(raw.clone());
        let base = if is_safe_sandbox_base(&normalized) {
            normalized
        } else if is_safe_sandbox_base(&raw) {
            raw
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "SWEEPER_SANDBOX_BASE is set but unsafe for sandboxing: '{}'. \
Set SWEEPER_SANDBOX_BASE to a safe directory outside protected container roots (not under Users/Windows/ProgramData, drive root, or browser User Data), \
or set CARGO_TARGET_DIR to a safe workspace target and retry.",
                    PathBuf::from(v).display()
                ),
            ));
        };
        let _ = fs::create_dir_all(&base);
        return Ok(base);
    }

    let temp = std::env::temp_dir();
    let canonical_temp = canonicalize_sandbox_base(temp.clone());
    if is_safe_sandbox_base(&canonical_temp) {
        return Ok(canonical_temp);
    }

    if let Some(base) = cargo_target_sandbox_base() {
        let _ = fs::create_dir_all(&base);
        let canonical_base = canonicalize_sandbox_base(base.clone());
        if is_safe_sandbox_base(&canonical_base) {
            return Ok(canonical_base);
        }
        if is_safe_sandbox_base(&base) {
            return Ok(base);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "no safe sandbox base found (temp '{}' is unsafe for sandboxing; no safe cargo-target fallback was available). \
Set SWEEPER_SANDBOX_BASE to a safe directory outside protected container roots, or set CARGO_TARGET_DIR to a safe workspace target and retry.",
            canonical_temp.display()
        ),
    ))
}

fn cargo_target_sandbox_base() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CARGO_TARGET_TMPDIR") {
        return Some(normalize_sandbox_base_path(
            PathBuf::from(v).join("sweeper-test-sandboxes"),
        ));
    }
    if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
        return Some(normalize_sandbox_base_path(
            PathBuf::from(v).join("sweeper-test-sandboxes"),
        ));
    }

    // Best-effort workspace-local fallback for testkit users outside cargo-target tmp contexts.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fallback = manifest.join("..\\..\\..\\target\\sweeper-test-sandboxes");
    Some(normalize_sandbox_base_path(fallback))
}

fn strip_extended_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    p
}

fn normalize_sandbox_base_path(base: PathBuf) -> PathBuf {
    // Ensure directories exist so canonicalization can resolve .. and remove short-name aliases.
    let _ = fs::create_dir_all(&base);
    if let Ok(c) = fs::canonicalize(&base) {
        return strip_extended_prefix(c);
    }
    // Fallback: lexical normalization (collapse ..) without touching FS beyond best effort.
    let normalized: PathBuf = base.components().collect();
    let _ = fs::create_dir_all(&normalized);
    if let Ok(c) = fs::canonicalize(&normalized) {
        return strip_extended_prefix(c);
    }
    strip_extended_prefix(normalized)
}

fn canonicalize_sandbox_base(base: PathBuf) -> PathBuf {
    normalize_sandbox_base_path(base)
}

/// Returns true if `path` is safe to use as a Sweeper test sandbox base.
///
/// A safe sandbox base must not contain short-name (`~`) or `..` components and must not be
/// located at or under protected container roots that would cause analyzer/policy to classify
/// sandbox contents as protected/unselectable (drive roots and `C:\Windows`, `C:\Users`,
/// `C:\ProgramData`, plus browser `User Data` container roots). Both normal and extended-length
/// (`\\?\`) canonical Windows path forms are handled, case-insensitively.
pub fn is_safe_sandbox_base(path: &Path) -> bool {
    !has_short_name_component(path)
        && !has_parent_dir_component(path)
        && !is_under_protected_container_root_for_sandbox(path)
}

fn is_under_protected_container_root_for_sandbox(path: &Path) -> bool {
    let normalized = strip_extended_prefix(path.to_path_buf());
    let parts: Vec<String> = normalized
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(os) => Some(os.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();

    // Drive root only (e.g. C:\) => no path segments after prefix/root.
    if parts.is_empty()
        && normalized
            .components()
            .any(|c| matches!(c, std::path::Component::RootDir))
    {
        return true;
    }

    // Match policy.rs protected container roots: exact or descendants of C:\Windows, C:\Users, C:\ProgramData.
    if !parts.is_empty() {
        let first = parts[0].to_ascii_uppercase();
        if matches!(first.as_str(), "WINDOWS" | "USERS" | "PROGRAMDATA") {
            return true;
        }
    }

    // Match policy.rs: paths with "User Data" segment are treated as under protected browser container roots.
    parts.iter().any(|s| s.eq_ignore_ascii_case("User Data"))
}

fn has_short_name_component(path: &Path) -> bool {
    path.components().any(|c| match c {
        std::path::Component::Normal(os) => os.to_string_lossy().contains('~'),
        _ => false,
    })
}

fn has_parent_dir_component(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

fn acquire_owner_lock(path: &Path) -> std::io::Result<fs::File> {
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

fn is_owner_lock_held(lock_path: &Path) -> bool {
    if !lock_path.exists() {
        return false;
    }
    use std::fs::OpenOptions;
    let mut opts = OpenOptions::new();
    opts.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.share_mode(0);
        match opts.open(lock_path) {
            Ok(_f) => {
                // Successfully acquired exclusive access => lock not held by a live owner.
                // (Dropping _f immediately releases; file may remain on disk.)
                false
            }
            Err(e) => {
                // Sharing violation / access denied while another process holds exclusive handle => live.
                matches!(e.raw_os_error(), Some(32 | 33 | 5))
                    || e.kind() == std::io::ErrorKind::PermissionDenied
            }
        }
    }
    #[cfg(not(windows))]
    {
        // No OS-enforced exclusive open lock on non-Windows in this testkit; treat existing lock file
        // conservatively as not provably live.
        let _ = opts.open(lock_path);
        false
    }
}

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
    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("attrib");
    cmd.creation_flags(CREATE_NO_WINDOW);
    // Clear all first, then set requested.
    cmd.arg("-R").arg("-H").arg("-S").arg(path);
    let _ = cmd.status();
    let mut cmd2 = std::process::Command::new("attrib");
    cmd2.creation_flags(CREATE_NO_WINDOW);
    if (attrs & FILE_ATTRIBUTE_READONLY) != 0 {
        cmd2.arg("+R");
    }
    if (attrs & FILE_ATTRIBUTE_HIDDEN) != 0 {
        cmd2.arg("+H");
    }
    if (attrs & FILE_ATTRIBUTE_SYSTEM) != 0 {
        cmd2.arg("+S");
    }
    // If no attrs requested, just ensure cleared.
    cmd2.arg(path);
    let status = cmd2.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(TestkitError::Io(std::io::Error::other(format!(
            "attrib failed for {}",
            path.display()
        ))))
    }
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

#[cfg(test)]
mod tests {
    use super::{
        has_parent_dir_component, has_short_name_component, is_safe_sandbox_base,
        normalize_sandbox_base_path,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn short_name_component_detection_flags_tilde_segments() {
        assert!(has_short_name_component(Path::new(
            r"C:\Temp\SHORT~1\AppData\Local\Temp"
        )));
        assert!(!has_short_name_component(Path::new(
            r"C:\Temp\longname\AppData\Local\Temp"
        )));
    }

    #[test]
    fn safe_sandbox_base_requires_no_short_name_segments() {
        assert!(!is_safe_sandbox_base(Path::new(
            r"C:\Temp\SHORT~1\AppData\Local\Temp"
        )));
        assert!(is_safe_sandbox_base(Path::new(
            r"D:\work\target\sweeper-test-sandboxes"
        )));
    }

    #[test]
    fn normalized_sandbox_base_has_no_parent_dir_components() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let p = manifest.join("..\\..\\..\\target\\sweeper-test-sandboxes");
        let norm = normalize_sandbox_base_path(p);
        assert!(
            !has_parent_dir_component(&norm),
            "normalized path must not contain '..' components: {}",
            norm.display()
        );
        assert!(
            !has_short_name_component(&norm),
            "normalized path must not contain short-name '~' segments: {}",
            norm.display()
        );
    }
}
