use std::path::Path;

use sweeper_testkit::{create_file_symlink, create_junction, CanaryTree, Sandbox};
use sweeper_winfs::{
    env_var, get_file_identity, get_final_path, inspect_reparse_point, open_for_enumeration,
    open_for_inspection, read_dir_entries, ReparseTagClass,
};

#[cfg(windows)]
#[test]
fn open_handle_does_not_follow_junction() {
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("junction-target").expect("canary");
    let inside = sandbox.create_dir("inside").expect("inside dir");
    let link = sandbox.root().join("junction_link");
    create_junction(&link, canary.root()).expect("create junction");

    // Opening the junction path itself must succeed as a link handle (no traversal into target).
    let handle = open_for_inspection(&link).expect("open junction handle");
    let final_path = get_final_path(&handle).expect("final path");
    let final_norm = normalize_path(&final_path);
    let link_norm = normalize_path(&link);
    assert_eq!(
        final_norm, link_norm,
        "final path should resolve to the junction link itself, not its target"
    );

    // Enumeration from a directory inside sandbox must not traverse external canary through junction.
    let entries = read_dir_entries(inside.parent().unwrap()).expect("read sandbox root");
    assert!(entries
        .iter()
        .any(|e| normalize_path(&e.path).ends_with("junction_link")));
    canary.verify().expect("external canary must remain intact");
    drop(handle);
}

/// The enumeration anchor deliberately omits `FILE_SHARE_DELETE`, so the pinned
/// directory entry cannot be removed or renamed while the handle is alive. This
/// is what closes the check-then-enumerate race: a swap cannot replace the
/// verified entry between inspection and the name-based `read_dir_entries` call.
// Test-only setup/teardown deletion; production removal stays confined to core::remover.
#[allow(clippy::disallowed_methods)]
#[cfg(windows)]
#[test]
fn enumeration_anchor_blocks_directory_swap_until_dropped() {
    let sandbox = Sandbox::new();
    let dir = sandbox.create_dir("anchor_pinned").expect("pinned dir");
    let anchor = open_for_enumeration(&dir).expect("enumeration anchor");

    // The anchor is still a usable reparse-safe inspection handle.
    let reparse = inspect_reparse_point(&anchor).expect("inspect anchor");
    assert!(matches!(reparse, ReparseTagClass::NotReparse));

    // The anchor holds DELETE access while denying delete share, so the swap
    // opener's share mode conflicts and removal fails with a sharing violation.
    let blocked = std::fs::remove_dir(&dir);
    let code = blocked.unwrap_err().raw_os_error().unwrap_or(0) as u32;
    assert!(
        sweeper_winfs::is_sharing_violation(code),
        "removal must fail with a sharing violation while the anchor is held (code {code})"
    );

    // Once the anchor is gone the same removal succeeds.
    drop(anchor);
    std::fs::remove_dir(&dir).expect("removal must succeed after the anchor is dropped");
    assert!(!dir.exists());
}

#[cfg(windows)]
#[test]
fn open_handle_does_not_follow_file_symlink() {
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("symlink-target").expect("canary");
    let target_file = canary.root().join("root.txt");
    let link = sandbox.root().join("file_link.txt");

    match create_file_symlink(&link, &target_file) {
        Ok(()) => {}
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("privilege not held") || msg.contains("permission") {
                eprintln!("skipping symlink test due to insufficient privilege: {e}");
                return;
            }
            panic!("create file symlink failed: {e}");
        }
    }

    let handle = open_for_inspection(&link).expect("open symlink handle");
    let final_path = get_final_path(&handle).expect("final path");
    assert_eq!(
        normalize_path(&final_path),
        normalize_path(&link),
        "file symlink handle must resolve to link path, not target"
    );
    canary.verify().expect("external canary must remain intact");
    drop(handle);
}

#[test]
fn reparse_classification_and_dir_entry_metadata() {
    let sandbox = Sandbox::new();
    let regular = sandbox
        .write_file("regular.txt", b"abc")
        .expect("write regular");
    let dir = sandbox.create_dir("plain_dir").expect("create dir");
    let canary = CanaryTree::seed_external("reparse-target").expect("canary");
    let junction = sandbox.root().join("j");
    #[cfg(windows)]
    create_junction(&junction, canary.root()).expect("create junction");

    let entries = read_dir_entries(sandbox.root()).expect("read dir");
    let regular_entry = find_entry(&entries, &regular).expect("regular entry");
    assert!(matches!(regular_entry.reparse, ReparseTagClass::NotReparse));
    assert!(!regular_entry.is_dir);
    assert_eq!(regular_entry.size, 3);

    let dir_entry = find_entry(&entries, &dir).expect("dir entry");
    assert!(dir_entry.is_dir);
    assert!(matches!(dir_entry.reparse, ReparseTagClass::NotReparse));

    #[cfg(windows)]
    {
        let junction_entry = find_entry(&entries, &junction).expect("junction entry");
        assert!(matches!(junction_entry.reparse, ReparseTagClass::LinkPoint));
        assert!(junction_entry.is_dir || junction_entry.reparse != ReparseTagClass::NotReparse);
    }

    canary.verify().expect("canary intact");
}

#[test]
fn canonical_path_and_file_identity_via_handle() {
    let sandbox = Sandbox::new();
    let file = sandbox
        .write_file("id.txt", b"identity")
        .expect("write file");
    let handle = open_for_inspection(&file).expect("open handle");
    let final_path = get_final_path(&handle).expect("final path");
    assert!(
        normalize_path(&final_path).ends_with("id.txt"),
        "canonical path should end with file name: {final_path}"
    );
    let id = get_file_identity(&handle).expect("file identity");
    assert!(
        id.volume_serial != 0 || id.file_index != 0,
        "identity should be populated: {id:?}"
    );
    drop(handle);
}

#[test]
fn directory_enumeration_records_link_points_without_traversing_external_canary() {
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("enum-canary").expect("canary");
    let safe_sub = sandbox.create_dir("safe").expect("safe dir");
    sandbox
        .write_file("safe/child.txt", b"child")
        .expect("write child");
    #[cfg(windows)]
    {
        let link = sandbox.root().join("escape");
        create_junction(&link, canary.root()).expect("create junction");
    }

    let entries = read_dir_entries(sandbox.root()).expect("enumerate sandbox");
    let names: Vec<String> = entries
        .iter()
        .map(|e| e.name.to_string_lossy().to_string())
        .collect();
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case("safe")));
    #[cfg(windows)]
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case("escape")));

    // Enumerating the sandbox must not require or produce entries from inside the canary target.
    assert!(
        !names.iter().any(|n| n.eq_ignore_ascii_case("root.txt")),
        "must not enumerate contents of external canary target"
    );

    // But enumerating the normal child directory works.
    let safe_entries = read_dir_entries(&safe_sub).expect("enumerate safe subdir");
    assert!(safe_entries
        .iter()
        .any(|e| e.name.to_string_lossy().eq_ignore_ascii_case("child.txt")));

    canary.verify().expect("external canary must remain intact");
}

#[test]
fn env_var_helper_reads_existing_var() {
    // PATH should exist on Windows; use a lenient assertion.
    let path = env_var("PATH");
    #[cfg(windows)]
    assert!(
        path.as_deref().map(|s| !s.is_empty()).unwrap_or(false),
        "PATH should be readable"
    );
    #[cfg(not(windows))]
    let _ = path;
}

fn find_entry<'a>(
    entries: &'a [sweeper_winfs::DirEntry],
    path: &Path,
) -> Option<&'a sweeper_winfs::DirEntry> {
    let want = normalize_path(path);
    entries.iter().find(|e| normalize_path(&e.path) == want)
}

fn normalize_path(p: impl AsRef<std::ffi::OsStr>) -> String {
    let s = p.as_ref().to_string_lossy().to_string();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    s.replace('/', "\\").to_lowercase()
}
