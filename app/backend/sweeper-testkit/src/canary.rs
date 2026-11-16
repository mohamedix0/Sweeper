use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use walkdir::WalkDir;

use crate::cleanup::{
    cleanup_stale_canaries, has_canary_markers, remove_dir_all_sandbox_root, CANARY_DIR_PREFIX,
};
use crate::owner_lock::{acquire_owner_lock, OWNER_LOCK_FILE_NAME};
use crate::{Result, TestkitError};

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
        seed_canary_files(&root)?;
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
        verify_manifest_entries(&self.manifest, &current)?;
        verify_no_unexpected_files(&self.manifest, &current)
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

fn seed_canary_files(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("nested"))?;
    write_bytes(root.join("root.txt"), b"canary-root")?;
    write_bytes(root.join("nested").join("file.txt"), b"canary-nested")?;
    write_bytes(root.join("nested").join("data.bin"), &[0, 1, 2, 3, 4, 5])?;
    Ok(())
}

fn verify_manifest_entries(
    expected: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> Result<()> {
    for (rel, expected_hash) in expected {
        let Some(found) = current.get(rel) else {
            return Err(TestkitError::CanaryMissing(rel.clone()));
        };
        if found != expected_hash {
            return Err(TestkitError::CanaryMismatch {
                path: rel.clone(),
                expected: expected_hash.clone(),
                found: found.clone(),
            });
        }
    }
    Ok(())
}

fn verify_no_unexpected_files(
    expected: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> Result<()> {
    for rel in current.keys() {
        if !expected.contains_key(rel) {
            return Err(TestkitError::CanaryUnexpected(rel.clone()));
        }
    }
    Ok(())
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
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.file_name() == OWNER_LOCK_FILE_NAME {
            continue;
        }
        insert_manifest_entry(root, entry.path(), &mut manifest)?;
    }
    Ok(manifest)
}

fn insert_manifest_entry(
    root: &Path,
    path: &Path,
    manifest: &mut BTreeMap<String, String>,
) -> Result<()> {
    let rel = path
        .strip_prefix(root)
        .map_err(|e| TestkitError::Walk(e.to_string()))?
        .to_string_lossy()
        .replace('\\', "/");
    let hash = sha256_file(path)?;
    manifest.insert(rel, hash);
    Ok(())
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
