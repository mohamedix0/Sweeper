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
    ///
    /// Prefers a canonicalized process temp directory so that Windows short-name aliases in
    /// environment paths (for example `RUNNER~1` on GitHub Actions) are expanded to long names
    /// while preserving temp-area semantics required by allowlist-sensitive tests (e.g. Gate F).
    /// Falls back to a cargo-target sandbox base only when canonicalized temp is unavailable or
    /// still contains short-name segments.
    pub fn new() -> Result<Self> {
        Self::new_in(default_sandbox_base())
    }

    /// Create a new isolated sandbox under `base_dir` with a sentinel file at its root.
    ///
    /// Useful for tests that must satisfy path-policy constraints that reject locations
    /// under protected container roots (e.g. `%TEMP%` inside a user profile on Windows).
    pub fn new_in(base_dir: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(base_dir.as_ref())?;
        let dir = tempfile::Builder::new()
            .prefix("sweeper-sandbox-")
            .tempdir_in(base_dir)?;
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

fn default_sandbox_base() -> PathBuf {
    let temp = std::env::temp_dir();
    let canonical_temp = canonicalize_sandbox_base(temp.clone());
    if !has_short_name_component(&canonical_temp) {
        return canonical_temp;
    }

    if let Some(base) = cargo_target_sandbox_base() {
        let _ = fs::create_dir_all(&base);
        let canonical_base = canonicalize_sandbox_base(base.clone());
        if is_safe_sandbox_base(&canonical_base) {
            return canonical_base;
        }
        if is_safe_sandbox_base(&base) {
            return base;
        }
    }

    // Last resort: preserve behavior even if no known-safe base is available.
    canonical_temp
}

fn cargo_target_sandbox_base() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("CARGO_TARGET_TMPDIR") {
        return Some(PathBuf::from(v).join("sweeper-test-sandboxes"));
    }
    if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
        return Some(PathBuf::from(v).join("sweeper-test-sandboxes"));
    }

    // Best-effort workspace-local fallback for testkit users outside cargo-target tmp contexts.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Some(
        manifest
            .join("..\\..\\..\\target\\sweeper-test-sandboxes")
            .components()
            .collect::<PathBuf>(),
    )
}

fn canonicalize_sandbox_base(base: PathBuf) -> PathBuf {
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

    let _ = fs::create_dir_all(&base);
    match fs::canonicalize(&base) {
        Ok(c) => strip_extended_prefix(c),
        Err(_) => base,
    }
}

fn is_safe_sandbox_base(path: &Path) -> bool {
    !has_short_name_component(path)
}

fn has_short_name_component(path: &Path) -> bool {
    path.components().any(|c| match c {
        std::path::Component::Normal(os) => os.to_string_lossy().contains('~'),
        _ => false,
    })
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
