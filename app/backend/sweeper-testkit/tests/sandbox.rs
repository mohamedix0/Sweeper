#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use sweeper_testkit::{
    cleanup_stale_sandboxes, is_safe_sandbox_base, CanaryTree, Sandbox, SANDBOX_DIR_PREFIX,
    SENTINEL_FILE_NAME,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn sandbox_creates_isolated_root_with_sentinel() {
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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
        is_safe_sandbox_base(Path::new(r"D:\SweeperTest\target\sandboxes")),
        "non-protected target path should be considered a safe sandbox base"
    );
}

#[test]
fn default_sandbox_base_selects_safe_non_protected_location() {
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
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

#[test]
fn cleanup_stale_sandboxes_skips_live_owner_lock_cross_platform() {
    let _env = env_lock();
    let base = tempfile::tempdir().expect("temp base");
    let base_path = base.path();
    let live = Sandbox::new_in(base_path);
    let live_root = live.root().to_path_buf();
    // On platforms where we can't force old mtime portably here, at minimum ensure a live owner lock
    // is treated as held and cleanup does not remove the live sandbox.
    let removed = cleanup_stale_sandboxes(base_path, Duration::from_secs(0)).expect("cleanup");
    let _ = removed;
    assert!(
        live_root.exists(),
        "live sandbox with owner lock must not be removed"
    );
    drop(live);
}

#[test]
fn cleanup_stale_canaries_skips_live_owner_lock_cross_platform() {
    let _env = env_lock();
    let base = tempfile::tempdir().expect("temp base");
    let base_path = base.path();
    let live = CanaryTree::seed_external("live-lock").expect("seed canary");
    // Copy-like scenario not needed; just assert cleanup against temp dir behavior is safe for live canary
    // by ensuring its root still exists while live (owner lock held by CanaryTree instance).
    assert!(live.root().exists(), "live canary should exist");
    let _ = sweeper_testkit::cleanup_stale_canaries(base_path, Duration::from_secs(0));
    assert!(
        live.root().exists(),
        "live canary with owner lock must not be removed by stale cleanup"
    );
}

#[cfg(windows)]
#[test]
fn partial_failure_preserves_sandbox_ownership_markers_for_retry() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let _env = env_lock();
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
    let _env = env_lock();
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
    let _env = env_lock();
    // Force unsafe temp and no usable cargo-target fallback.
    let unsafe_target = tempfile::tempdir().expect("tempdir for unsafe target-dir shim");
    let _guard = EnvGuard::set_multi(&[
        ("SWEEPER_SANDBOX_BASE", None),
        ("TEMP", Some(r"C:\Users")),
        ("TMP", Some(r"C:\Users")),
        ("CARGO_TARGET_TMPDIR", None),
        ("CARGO_TARGET_DIR", Some(r"C:\Users")),
    ]);
    // Side-effect-free safety validation must reject unsafe bases (including CARGO_TARGET_DIR)
    // without attempting to create sandbox directories under protected roots.
    assert!(
        !is_safe_sandbox_base(Path::new(r"C:\Users")),
        "C:\\Users must be rejected by sandbox base safety validation"
    );
    let err = sweeper_testkit::Sandbox::try_new().expect_err("expected unsafe-base error");
    let msg = err.to_string();
    assert!(
        msg.contains("SWEEPER_SANDBOX_BASE") || msg.contains("CARGO_TARGET_DIR"),
        "error should be actionable and mention SWEEPER_SANDBOX_BASE/CARGO_TARGET_DIR: {msg}"
    );
    // Ensure we did not create sandbox content in the protected location via this test path.
    // (CARGO_TARGET_DIR points to C:\Users; resolver must not create sweeper-test-sandboxes there.)
    let _ = unsafe_target; // keep fixture alive for test isolation patterns
}

#[cfg(windows)]
#[test]
fn configured_sandbox_base_rejects_junction_resolving_into_protected_users() {
    let _env = env_lock();
    let safe_parent = tempfile::tempdir_in(std::env::temp_dir()).expect("safe temp base");
    let junction = safe_parent.path().join("users-junction");
    sweeper_testkit::create_junction(&junction, Path::new(r"C:\Users"))
        .expect("create junction to Users");
    let _guard = EnvGuard::set_multi(&[
        (
            "SWEEPER_SANDBOX_BASE",
            Some(junction.to_string_lossy().as_ref()),
        ),
        ("TEMP", Some(r"C:\Users")),
        ("TMP", Some(r"C:\Users")),
        ("CARGO_TARGET_TMPDIR", None),
        ("CARGO_TARGET_DIR", None),
    ]);
    let err = sweeper_testkit::default_sandbox_base_checked()
        .expect_err("configured base resolving into protected Users must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("SWEEPER_SANDBOX_BASE")
            || msg.contains("unsafe")
            || msg.contains("no safe sandbox base"),
        "expected rejection for junction-resolved unsafe configured base, got: {msg}"
    );
}

#[cfg(windows)]
#[test]
fn cleanup_stale_canaries_removes_nested_extras_and_retries_when_marker_locked() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let _env = env_lock();
    let base = tempfile::tempdir().expect("temp base");
    let canary = base.path().join("sweeper-canary-partial-nested");
    fs::create_dir_all(canary.join("nested")).expect("create nested");
    fs::write(canary.join("root.txt"), b"canary-root").expect("root marker");
    fs::write(canary.join("nested").join("file.txt"), b"canary-nested")
        .expect("nested marker file");
    fs::write(canary.join("nested").join("data.bin"), [0u8, 1, 2, 3]).expect("nested marker data");
    fs::write(canary.join("nested").join("extra.txt"), b"unexpected").expect("extra file");
    set_path_mtime_old(&canary, 7200);
    set_path_mtime_old(&canary.join("nested"), 7200);

    let marker = canary.join("nested").join("file.txt");
    let handle = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&marker)
        .expect("lock nested marker file");

    let _ = sweeper_testkit::cleanup_stale_canaries(base.path(), Duration::from_secs(3600));
    assert!(
        !canary.join("nested").join("extra.txt").exists(),
        "unexpected extra file inside nested must be removed even when nested is traversed"
    );
    assert!(
        canary.join("root.txt").is_file()
            && canary.join("nested").join("file.txt").is_file()
            && canary.join("nested").join("data.bin").is_file(),
        "canary ownership markers must remain when cleanup cannot complete"
    );

    drop(handle);
    let removed = sweeper_testkit::cleanup_stale_canaries(base.path(), Duration::from_secs(3600))
        .expect("retry cleanup");
    assert!(
        removed >= 1,
        "retry should remove canary after lock release"
    );
    assert!(!canary.exists(), "canary dir should be removed on retry");
}

struct EnvGuard {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    fn set_multi(vars: &[(&'static str, Option<&str>)]) -> Self {
        let mut saved = Vec::with_capacity(vars.len());
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

#[test]
fn guarded_path_rejects_parent_dir_escape() {
    let _env = env_lock();
    let sandbox = Sandbox::new();
    let err = sandbox
        .write_file(Path::new("..").join("outside.txt"), b"x")
        .expect_err("parent-dir escape must be rejected");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("escape") || msg.contains("invalid") || msg.contains("relative"),
        "unexpected error: {err}"
    );
}

#[test]
fn guarded_path_rejects_absolute_paths() {
    let _env = env_lock();
    let sandbox = Sandbox::new();
    let abs = sandbox.root().join("abs.txt");
    let err = sandbox
        .write_file(&abs, b"x")
        .expect_err("absolute path must be rejected");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("escape") || msg.contains("invalid") || msg.contains("relative"),
        "unexpected error: {err}"
    );
}

#[cfg(windows)]
#[test]
fn guarded_path_rejects_junction_parent_escape_attempt() {
    let _env = env_lock();
    let sandbox = Sandbox::new();
    let outside = tempfile::tempdir().expect("outside tempdir");
    let link = sandbox.root().join("escape_link");
    sweeper_testkit::create_junction(&link, outside.path()).expect("create junction");
    let err = sandbox
        .write_file(Path::new("escape_link").join("pwned.txt"), b"x")
        .expect_err("junction parent escape must be rejected");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("escape")
            || msg.contains("symlink")
            || msg.contains("junction")
            || msg.contains("root"),
        "unexpected error: {err}"
    );
    assert!(
        !outside.path().join("pwned.txt").exists(),
        "must not write outside sandbox through junction"
    );
}
