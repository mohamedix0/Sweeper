#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup; production removal is enforced in core::remover.

use std::fs;
use std::time::Duration;

use sweeper_core::remover::{self, CleanOptions};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

/// D1: an interior Link Point is invisible to the OS-resolved final path.
///
/// Preview confirms `root/sub/victim.txt`. Between enumeration and Clean, the
/// ordinary directory `sub` is swapped for a junction to the sibling `root/keep`
/// (the same authorized root, so containment still passes). `open_for_delete`
/// follows the *intermediate* junction because `FILE_FLAG_OPEN_REPARSE_POINT`
/// protects only the final component, and `GetFinalPathNameByHandle` then reports
/// `root/keep/victim.txt` with the junction erased from the chain. Files in
/// `keep`, never part of the confirmed selection, must not be deleted.
#[test]
fn gate_d_interior_junction_redirect_is_detected_and_siblings_survive() {
    let sb = Sandbox::new();
    let target_root = sb.create_dir("target").unwrap();
    let journal_dir = sb.create_dir("journal").unwrap();

    let sub = target_root.join("sub");
    let keep = target_root.join("keep");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(&keep).unwrap();

    // Confirmed selection captured while `sub` is still an ordinary directory.
    let victim = sub.join("victim.txt");
    write_file(&victim, b"selected for deletion");
    set_filetime(&victim, old_mtime_secs());
    set_filetime(&sub, old_mtime_secs());
    let item = make_item("interior_junction", &target_root, &victim, 1024);

    // Sibling files inside the same root were never selected. The file with the
    // same name is what `root/sub/victim.txt` resolves to through the junction.
    let sibling_victim = keep.join("victim.txt");
    let sibling_guard = keep.join("survivor.txt");
    write_file(&sibling_victim, b"never selected");
    write_file(&sibling_guard, b"never selected");
    set_filetime(&sibling_victim, old_mtime_secs());
    set_filetime(&sibling_guard, old_mtime_secs());

    // TOCTOU swap: replace the confirmed directory with a junction to the sibling.
    fs::remove_file(&victim).unwrap();
    fs::remove_dir(&sub).unwrap();
    sweeper_testkit::create_junction(&sub, &keep).unwrap();

    let report = remover::execute_clean(
        vec![item],
        CleanOptions {
            preview_estimated_bytes: 1024 * 1024,
            min_age: Duration::from_secs(3600),
            now: now(),
            journal_base_dir: Some(journal_dir.clone()),
        },
    )
    .unwrap();

    assert!(
        report.aborted,
        "interior-junction redirect must abort the clean, report={report:?}"
    );
    assert_eq!(
        report.bytes_freed, 0,
        "no unselected sibling bytes may be freed, report={report:?}"
    );
    assert!(
        sibling_victim.exists(),
        "unselected sibling victim.txt must survive"
    );
    assert!(
        sibling_guard.exists(),
        "unselected sibling survivor.txt must survive"
    );
    assert_sentinel_intact(&sb);
}
