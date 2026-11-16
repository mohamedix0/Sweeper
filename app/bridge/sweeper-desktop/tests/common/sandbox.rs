#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

pub fn sandbox_test_base() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target-fallback"))
        .join("sweeper-test-sandboxes");
    let _ = fs::create_dir_all(&base);
    match fs::canonicalize(&base) {
        Ok(c) => normalize_canonical_path(&c),
        Err(_) => base.components().collect::<PathBuf>(),
    }
}

fn normalize_canonical_path(c: &Path) -> PathBuf {
    let s = c.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    c.to_path_buf()
}
