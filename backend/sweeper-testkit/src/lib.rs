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
use tempfile::TempDir;
use thiserror::Error;
use walkdir::WalkDir;

/// Sentinel file name required at the root of every deletion/test sandbox.
pub const SENTINEL_FILE_NAME: &str = ".sweeper-sandbox-sentinel";

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
    _dir: TempDir,
    root: PathBuf,
}

impl Sandbox {
    /// Create a new isolated sandbox with a sentinel file at its root.
    pub fn new() -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("sweeper-sandbox-")
            .tempdir()?;
        let root = dir.path().to_path_buf();
        let sentinel = root.join(SENTINEL_FILE_NAME);
        let mut f = fs::File::create(&sentinel)?;
        writeln!(f, "sweeper-testkit sentinel; do not remove during test run")?;
        f.sync_all()?;
        Ok(Self { _dir: dir, root })
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

/// External canary tree with SHA-256 manifest for post-run integrity verification.
#[derive(Debug)]
pub struct CanaryTree {
    _dir: TempDir,
    root: PathBuf,
    manifest: BTreeMap<String, String>,
}

impl CanaryTree {
    /// Seed an external canary tree (outside any test sandbox) with deterministic files.
    pub fn seed_external(label: &str) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix(&format!("sweeper-canary-{label}-"))
            .tempdir()?;
        let root = dir.path().to_path_buf();

        // Seed a small deterministic tree.
        fs::create_dir_all(root.join("nested"))?;
        write_bytes(root.join("root.txt"), b"canary-root")?;
        write_bytes(root.join("nested").join("file.txt"), b"canary-nested")?;
        write_bytes(root.join("nested").join("data.bin"), &[0, 1, 2, 3, 4, 5])?;

        let manifest = build_manifest(&root)?;
        Ok(Self {
            _dir: dir,
            root,
            manifest,
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

/// Create a directory junction at `link` pointing to `target` (Windows).
#[cfg(windows)]
pub fn create_junction(link: &Path, target: &Path) -> Result<()> {
    let target_abs = fs::canonicalize(target)?;
    let link_arg = link.to_string_lossy();
    let target_arg = target_abs.to_string_lossy();
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J", &*link_arg, &*target_arg])
        .status()
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!(
                    "mklink /J failed with status {status} for link '{}' -> target '{}'",
                    link.display(),
                    target_abs.display()
                )))
            }
        })?;
    Ok(())
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
