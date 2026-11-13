use std::fs;
use std::path::Path;

use sweeper_testkit::{CanaryTree, Sandbox, SENTINEL_FILE_NAME};

#[test]
fn sandbox_creates_isolated_root_with_sentinel() {
    let sandbox = Sandbox::new().expect("sandbox should be created");
    let root = sandbox.root();
    assert!(root.exists(), "sandbox root must exist");
    assert!(root.join(SENTINEL_FILE_NAME).is_file(), "sentinel file must exist at sandbox root");
}

#[test]
fn sandbox_refuses_operation_when_sentinel_missing() {
    let sandbox = Sandbox::new().expect("sandbox should be created");
    let sentinel = sandbox.root().join(SENTINEL_FILE_NAME);
    fs::remove_file(&sentinel).expect("remove sentinel for test");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sandbox.assert_sentinel_present();
    }));
    assert!(result.is_err(), "missing sentinel must cause panic/refusal");
}

#[test]
fn canary_tree_seed_and_verify_hashes_outside_sandbox() {
    let sandbox = Sandbox::new().expect("sandbox should be created");
    // Canary lives outside the sandbox target area (separate temp root).
    let canary = CanaryTree::seed_external("phase0-canary").expect("seed canary");
    assert!(!path_starts_with(canary.root(), sandbox.root()), "canary must live outside sandbox root");

    canary.verify().expect("fresh canary should verify");

    // Tamper with a file and ensure verification detects it.
    let file = canary.root().join("nested").join("file.txt");
    fs::write(&file, b"tampered").expect("tamper canary");
    let err = canary.verify().expect_err("tampered canary must fail verification");
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("hash") || msg.contains("mismatch") || msg.contains("canary"), "error should mention hash/mismatch: {err}");
}

fn path_starts_with(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

#[cfg(windows)]
#[test]
fn junction_helper_creates_link_point_inside_sandbox() {
    let sandbox = Sandbox::new().expect("sandbox should be created");
    let target = sandbox.root().join("target_dir");
    let link = sandbox.root().join("link_dir");
    fs::create_dir_all(&target).expect("create target dir");
    fs::write(target.join("hello.txt"), b"hi").expect("write file");

    sweeper_testkit::create_junction(&link, &target).expect("create junction");

    let meta = fs::symlink_metadata(&link).expect("link metadata");
    assert!(meta.file_type().is_symlink() || meta.is_dir(), "junction should exist");
    // Reading through junction should reach target content.
    let content = fs::read_to_string(link.join("hello.txt")).expect("read through junction");
    assert_eq!(content, "hi");
}

#[cfg(windows)]
#[test]
fn symlink_file_helper_creates_symlink_inside_sandbox() {
    let sandbox = Sandbox::new().expect("sandbox should be created");
    let target = sandbox.root().join("target.txt");
    let link = sandbox.root().join("link.txt");
    fs::write(&target, b"payload").expect("write target");

    sweeper_testkit::create_file_symlink(&link, &target).expect("create file symlink");

    let meta = fs::symlink_metadata(&link).expect("link metadata");
    assert!(meta.file_type().is_symlink(), "file symlink should be created");
    let content = fs::read_to_string(&link).expect("read via symlink");
    assert_eq!(content, "payload");
}
