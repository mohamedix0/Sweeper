#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, ValidatedSource};
use sweeper_testkit::Sandbox;

pub fn now() -> SystemTime {
    SystemTime::now()
}

#[allow(dead_code)] // shared helper; not every test binary uses it
pub fn unix_now() -> u64 {
    now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

pub fn old_mtime_secs() -> u64 {
    unix_now().saturating_sub(48 * 3600)
}

pub fn canonical_str(p: &Path) -> String {
    CanonicalPath::from_existing_path(&p.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

#[allow(dead_code)] // shared helper; not every test binary uses it
pub fn path_str_for_item(p: &Path) -> String {
    if p.exists() {
        // Link points are valid deletion targets as entries; from_existing_path rejects them.
        if let Ok(v) = CanonicalPath::from_existing_path(&p.to_string_lossy()) {
            return v.as_str().to_string();
        }
    }
    CanonicalPath::parse_lexical(&p.to_string_lossy())
        .unwrap()
        .as_str()
        .to_string()
}

#[allow(dead_code)] // shared helper; not every test binary uses it
pub fn make_item(id: &str, target_root: &Path, path: &Path, size: u64) -> remover::ValidatedItem {
    let root = canonical_str(target_root);
    let p = path_str_for_item(path);
    remover::validated_item_for_tests(
        id,
        "test-target",
        root,
        p,
        size,
        Some(old_mtime_secs()),
        ValidatedSource::CustomPath,
    )
}

#[allow(dead_code)] // shared helper; not every test binary uses it
pub fn clean_one(
    item: remover::ValidatedItem,
    preview_estimated_bytes: u64,
    min_age: Duration,
    journal_base: &Path,
) -> remover::CleanReport {
    remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes,
            min_age,
            now: now(),
            journal_base_dir: Some(journal_base.to_path_buf()),
        },
    )
    .unwrap()
}

pub fn write_file(path: &Path, contents: &[u8]) {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(path, contents).unwrap();
}

pub fn set_filetime(path: &Path, secs: u64) {
    let ft = sweeper_testkit::filetime_from_unix_secs(secs);
    sweeper_testkit::set_last_write_time(path, ft).unwrap();
}

pub fn assert_sentinel_intact(sb: &Sandbox) {
    sb.assert_sentinel_present();
}
