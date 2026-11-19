#![cfg(windows)]
#![allow(clippy::disallowed_methods)] // Test-only setup/cleanup; production removal is enforced in core::remover.

use std::fs;
use std::io::Write;
use std::time::Duration;

use sweeper_core::journal::{self, JournalRecord};
use sweeper_testkit::Sandbox;

mod common;
use common::*;

#[test]
fn gate_d_mid_clean_termination_recovery_truncated_journal_and_rotation() {
    let sb = Sandbox::new();
    let journal_base = sb.create_dir("logsbase").unwrap();
    let log_dir = journal::resolve_log_dir(&journal_base);
    fs::create_dir_all(&log_dir).unwrap();

    // Create 35 log files to verify rotation keeps last 30.
    for i in 0..35u64 {
        let p = log_dir.join(format!("clean-170000{:04}-{}-0.jsonl", i, i));
        let mut f = fs::File::create(&p).unwrap();
        writeln!(f, "{}", serde_json::json!({"record":"intent","target_id":"t","path":"C:\\\\a\\\\b\\\\c","size":1,"timestamp":1700000000u64+i})).unwrap();
        // set distinct modified times by rewriting; best effort
    }
    journal::rotate_logs(&log_dir, 30).unwrap();
    let count = fs::read_dir(&log_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .count();
    assert!(
        count <= 30,
        "rotation should retain last 30 logs, got {count}"
    );

    // Simulate truncated journal (mid-clean termination) and ensure reader recovers.
    let truncated = log_dir.join("clean-1700999999-1-0.jsonl");
    {
        let mut f = fs::File::create(&truncated).unwrap();
        writeln!(
            f,
            "{}",
            serde_json::to_string(&JournalRecord::Intent {
                target_id: "x".into(),
                path: "C:\\Temp\\x".into(),
                size: 1,
                timestamp: 1,
                batch_id: None
            })
            .unwrap()
        )
        .unwrap();
        // truncated/incomplete trailing line
        write!(f, "{{\"record\":\"outcome\",\"target_id\":\"x\"").unwrap();
    }
    let recs = journal::read_records(&truncated).unwrap();
    assert_eq!(recs.len(), 1, "truncated trailing line should be ignored");
    match &recs[0] {
        JournalRecord::Intent { .. } => {}
        _ => panic!("expected intent record"),
    }

    // Also run a small clean to produce a valid journal and ensure logs exist.
    let target_root = sb.create_dir("target").unwrap();
    let victim = target_root.join("v.txt");
    write_file(&victim, b"bye");
    set_filetime(&victim, old_mtime_secs());
    let item = make_item("i", &target_root, &victim, 3);
    let _ = clean_one(item, 1024, Duration::from_secs(3600), &journal_base);
    assert!(!victim.exists());
    assert_sentinel_intact(&sb);
}
