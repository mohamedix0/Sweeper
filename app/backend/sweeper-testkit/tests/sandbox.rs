#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sweeper_testkit::{
    cleanup_stale_sandboxes, is_safe_sandbox_base, CanaryTree, Sandbox, SANDBOX_DIR_PREFIX,
    SENTINEL_FILE_NAME,
};

#[test]
fn sandbox_creates_isolated_root_with_sentinel() {
    let sandbox = Sandbox::new();
    let root = sandbox.root();
    assert!(root.exists(), "sandbox root must exist");
    assert!(
        root.join(SENTINEL_FILE_NAME).is_file(),
        "sentinel file must exist at sandbox root"
    );
}

#[test]
fn sandbox_refuses_operation_when_sentinel_missing() {
    let sandbox = Sandbox::new();
    let sentinel = sandbox.root().join(SENTINEL_FILE_NAME);
    #[allow(clippy::disallowed_methods)]
    fs::remove_file(&sentinel).expect("remove sentinel for test");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sandbox.assert_sentinel_present();
    }));
    assert!(result.is_err(), "missing sentinel must cause panic/refusal");
}

#[test]
fn canary_tree_seed_and_verify_hashes_outside_sandbox() {
    let sandbox = Sandbox::new();
    // Canary lives outside the sandbox target area (separate temp root).
    let canary = CanaryTree::seed_external("phase0-canary").expect("seed canary");
    assert!(
        !path_starts_with(canary.root(), sandbox.root()),
        "canary must live outside sandbox root"
    );

    canary.verify().expect("fresh canary should verify");

    // Tamper with a file and ensure verification detects it.
    let file = canary.root().join("nested").join("file.txt");
    fs::write(&file, b"tampered").expect("tamper canary");
    let err = canary
        .verify()
        .expect_err("tampered canary must fail verification");
    match err {
        sweeper_testkit::TestkitError::CanaryMismatch { path, .. } => {
            assert_eq!(
                path, "nested/file.txt",
                "mismatch should report tampered file path"
            );
        }
        other => panic!("expected CanaryMismatch for tampered file, got: {other:?}"),
    }
}

fn path_starts_with(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

#[cfg(windows)]
fn to_extended_length_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") {
        path.to_path_buf()
    } else if let Some(rest) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{}", rest))
    } else {
        PathBuf::from(format!(r"\\?\{}", s))
    }
}

#[cfg(windows)]
fn create_deep_tree_extended(root: &Path, depth: usize) -> PathBuf {
    let mut cur = to_extended_length_path(root);
    fs::create_dir_all(&cur).expect("create extended root");
    for i in 0..depth {
        cur.push(format!("d{i:04}"));
        fs::create_dir(&cur).expect("create deep dir segment");
    }
    cur
}

#[cfg(windows)]
fn set_path_mtime_old(path: &Path, secs_ago: u64) {
    let now = sweeper_testkit::filetime_now();
    let delta = secs_ago.saturating_mul(10_000_000);
    let old = now.saturating_sub(delta);
    sweeper_testkit::set_last_write_time(path, old).expect("set old mtime");
}

#[cfg(windows)]
#[test]
fn sandbox_drop_cleans_deep_tree_with_extended_paths() {
    let sandbox = Sandbox::new();
    let root = sandbox.root().to_path_buf();
    let deep_leaf = create_deep_tree_extended(&root, 320);
    let deep_file = deep_leaf.join("leaf.txt");
    fs::write(&deep_file, b"deep").expect("write deep file");
    assert!(deep_file.exists(), "deep file should exist before drop");

    drop(sandbox);

    assert!(
        !root.exists(),
        "sandbox root with deep (>300) tree should be removed on drop: {}",
        root.display()
    );
}

#[cfg(windows)]
#[test]
fn cleanup_stale_sandboxes_respects_safety_guards() {
    let base = tempfile::tempdir().expect("temp base");
    let base_path = base.path();

    // (a) old sentinel-marked sweeper-sandbox-* dir is removed
    let old_ok = base_path.join(format!("{SANDBOX_DIR_PREFIX}old-ok"));
    fs::create_dir_all(&old_ok).expect("create old sandbox dir");
    fs::write(old_ok.join(SENTINEL_FILE_NAME), b"sentinel").expect("write sentinel");
    set_path_mtime_old(&old_ok, 7200);

    // (b) dir without sentinel is refused
    let old_no_sentinel = base_path.join(format!("{SANDBOX_DIR_PREFIX}old-no-sentinel"));
    fs::create_dir_all(&old_no_sentinel).expect("create no-sentinel dir");
    set_path_mtime_old(&old_no_sentinel, 7200);

    // (c) fresh dir younger than max_age is refused
    let fresh = base_path.join(format!("{SANDBOX_DIR_PREFIX}fresh"));
    fs::create_dir_all(&fresh).expect("create fresh dir");
    fs::write(fresh.join(SENTINEL_FILE_NAME), b"sentinel").expect("write sentinel");

    // (d) junction/reparse point with matching prefix is refused (and must not be followed)
    let outside = tempfile::tempdir().expect("outside target");
    let outside_keep = outside.path().join("keep.txt");
    fs::write(&outside_keep, b"keep").expect("write outside canary");
    let junction_dir = base_path.join(format!("{SANDBOX_DIR_PREFIX}junction"));
    sweeper_testkit::create_junction(&junction_dir, outside.path()).expect("create junction");

    let removed =
        cleanup_stale_sandboxes(base_path, Duration::from_secs(3600)).expect("cleanup succeeds");

    assert!(removed >= 1, "expected at least old sandbox removed");
    assert!(
        !old_ok.exists(),
        "old sentinel-marked sandbox should be removed"
    );
    assert!(
        old_no_sentinel.exists(),
        "dir without sentinel must not be removed"
    );
    assert!(
        fresh.exists(),
        "fresh dir younger than max_age must not be removed"
    );
    assert!(
        junction_dir.exists() || fs::symlink_metadata(&junction_dir).is_ok(),
        "junction/reparse point must not be removed"
    );
    assert!(
        outside_keep.exists(),
        "cleanup must not follow/remove through junction target"
    );
}

#[test]
fn is_safe_sandbox_base_rejects_protected_container_roots_and_users_paths() {
    for p in [
        r"C:\Users\alice\AppData\Local\Temp",
        r"c:\users\alice\AppData\Local\Temp",
        r"\\?\C:\Users\alice\AppData\Local\Temp",
        r"\\?\c:\users\alice\AppData\Local\Temp",
        r"C:\Users",
        r"\\?\C:\Users",
        r"C:\ProgramData\Temp",
        r"\\?\C:\ProgramData\Temp",
        r"C:\Windows\Temp",
        r"\\?\C:\Windows\Temp",
        r"C:\",
        r"\\?\C:\",
        r"C:\Users\alice\AppData\Local\Google\Chrome\User Data\Default",
        r"\\?\C:\Users\alice\AppData\Local\Google\Chrome\User Data\Default",
    ] {
        assert!(
            !is_safe_sandbox_base(Path::new(p)),
            "expected sandbox base to be rejected as protected/container-root path: {p}"
        );
    }

    // Non-protected, long-name path should remain acceptable (subject to filesystem context).
    assert!(
        is_safe_sandbox_base(Path::new(r"D:\work\target\sweeper-test-sandboxes")),
        "non-protected target path should be considered a safe sandbox base"
    );
}

#[test]
fn default_sandbox_base_selects_safe_non_protected_location() {
    let base = sweeper_testkit::default_sandbox_base();
    assert!(
        is_safe_sandbox_base(&base),
        "default_sandbox_base must return a safe sandbox base, got: {}",
        base.display()
    );
}

#[cfg(windows)]
#[test]
fn junction_helper_creates_link_point_inside_sandbox() {
    let sandbox = Sandbox::new();
    let target = sandbox.root().join("target_dir");
    let link = sandbox.root().join("link_dir");
    fs::create_dir_all(&target).expect("create target dir");
    fs::write(target.join("hello.txt"), b"hi").expect("write file");

    sweeper_testkit::create_junction(&link, &target).expect("create junction");

    let meta = fs::symlink_metadata(&link).expect("link metadata");
    assert!(
        meta.file_type().is_symlink() || meta.is_dir(),
        "junction should exist"
    );
    // Reading through junction should reach target content.
    let content = fs::read_to_string(link.join("hello.txt")).expect("read through junction");
    assert_eq!(content, "hi");
}

#[cfg(windows)]
#[test]
fn symlink_file_helper_creates_symlink_inside_sandbox() {
    let sandbox = Sandbox::new();
    let target = sandbox.root().join("target.txt");
    let link = sandbox.root().join("link.txt");
    fs::write(&target, b"payload").expect("write target");

    match sweeper_testkit::create_file_symlink(&link, &target) {
        Ok(()) => {
            let meta = fs::symlink_metadata(&link).expect("link metadata");
            assert!(
                meta.file_type().is_symlink(),
                "file symlink should be created"
            );
            let content = fs::read_to_string(&link).expect("read via symlink");
            assert_eq!(content, "payload");
        }
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("privilege not held") || msg.contains("permission") {
                eprintln!("skipping symlink assertion due to insufficient privilege: {e}");
            } else {
                panic!("create file symlink failed: {e}");
            }
        }
    }
}

#[cfg(windows)]
#[test]
fn sandbox_drop_disarms_tempdir_when_sentinel_missing() {
    let sandbox = Sandbox::new();
    let root = sandbox.root().to_path_buf();
    let sentinel = root.join(SENTINEL_FILE_NAME);
    fs::remove_file(&sentinel).expect("remove sentinel");
    drop(sandbox);
    assert!(
        root.exists(),
        "sandbox root must remain when sentinel is missing (TempDir auto-cleanup disarmed)"
    );
    // Best-effort manual cleanup for test hygiene.
    let _ = fs::remove_dir_all(&root);
}

#[cfg(windows)]
#[test]
fn cleanup_stale_sandboxes_skips_live_owner_lock() {
    let base = tempfile::tempdir().expect("temp base");
    let base_path = base.path();

    // Live sandbox holds exclusive owner lock and must not be removed even if old.
    let live = Sandbox::new_in(base_path);
    let live_root = live.root().to_path_buf();
    set_path_mtime_old(&live_root, 7200);

    let stale = base_path.join(format!("{SANDBOX_DIR_PREFIX}stale-unlocked"));
    fs::create_dir_all(&stale).expect("create stale dir");
    fs::write(stale.join(SENTINEL_FILE_NAME), b"sentinel").expect("write sentinel");
    set_path_mtime_old(&stale, 7200);

    let removed = cleanup_stale_sandboxes(base_path, Duration::from_secs(3600)).expect("cleanup");
    assert!(removed >= 1, "expected stale unlocked sandbox removed");
    assert!(
        live_root.exists(),
        "live locked sandbox must not be removed"
    );
    assert!(!stale.exists(), "stale unlocked sandbox should be removed");
    drop(live);
}

#[cfg(windows)]
#[test]
fn partial_failure_preserves_sandbox_ownership_markers_for_retry() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let base = tempfile::tempdir().expect("temp base");
    let stale = base.path().join(format!("{SANDBOX_DIR_PREFIX}partial"));
    fs::create_dir_all(&stale).expect("create stale dir");
    fs::write(stale.join(SENTINEL_FILE_NAME), b"sentinel").expect("write sentinel");
    let locked = stale.join("locked.txt");
    fs::write(&locked, b"locked").expect("write locked file");
    // Exclusive handle prevents deletion while held.
    let _handle = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&locked)
        .expect("lock file exclusively");
    set_path_mtime_old(&stale, 7200);

    let _ = cleanup_stale_sandboxes(base.path(), Duration::from_secs(3600));
    assert!(
        stale.join(SENTINEL_FILE_NAME).exists(),
        "sentinel/ownership marker must remain after partial cleanup failure"
    );
    assert!(
        locked.exists(),
        "locked file should remain after partial failure"
    );
}

#[cfg(windows)]
#[test]
fn junction_cleanup_unlinks_junction_without_touching_target() {
    let base = tempfile::tempdir().expect("temp base");
    let stale = base
        .path()
        .join(format!("{SANDBOX_DIR_PREFIX}junction-clean"));
    fs::create_dir_all(&stale).expect("create stale dir");
    fs::write(stale.join(SENTINEL_FILE_NAME), b"sentinel").expect("write sentinel");

    let outside = tempfile::tempdir().expect("outside target");
    let keep = outside.path().join("keep.txt");
    fs::write(&keep, b"keep").expect("write target file");
    sweeper_testkit::create_junction(&stale.join("link"), outside.path()).expect("create junction");
    set_path_mtime_old(&stale, 7200);

    let removed = cleanup_stale_sandboxes(base.path(), Duration::from_secs(3600)).expect("cleanup");
    assert!(
        removed >= 1,
        "stale sandbox with junction should be removed"
    );
    assert!(!stale.exists(), "sandbox root should be removed");
    assert!(
        keep.exists(),
        "junction target contents must not be deleted by cleanup"
    );
}

#[test]
fn try_new_returns_actionable_error_when_no_safe_base_exists() {
    // Force unsafe temp and no usable cargo-target fallback.
    let _guard = EnvGuard::set_multi(&[
        ("SWEEPER_SANDBOX_BASE", None),
        ("TEMP", Some(r"C:\Users")),
        ("TMP", Some(r"C:\Users")),
        ("CARGO_TARGET_TMPDIR", None),
        ("CARGO_TARGET_DIR", Some(r"C:\Users")),
    ]);
    let err = sweeper_testkit::Sandbox::try_new().expect_err("expected unsafe-base error");
    let msg = err.to_string();
    assert!(
        msg.contains("SWEEPER_SANDBOX_BASE") || msg.contains("CARGO_TARGET_DIR"),
        "error should be actionable and mention SWEEPER_SANDBOX_BASE/CARGO_TARGET_DIR: {msg}"
    );
}

struct EnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}
impl EnvGuard {
    fn set_multi(vars: &[(&'static str, Option<&str>)]) -> Self {
        let mut saved = Vec::new();
        for (k, v) in vars {
            saved.push((*k, std::env::var_os(k)));
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
        Self { saved }
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in self.saved.drain(..).rev() {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}
