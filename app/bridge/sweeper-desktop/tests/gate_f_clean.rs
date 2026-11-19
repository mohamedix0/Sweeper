mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sweeper_contract::{ContractError, U64String};
use sweeper_desktop::commands_impl;
use sweeper_testkit::{CanaryTree, Sandbox};

fn assert_preview_invalid(err: ContractError) {
    common::assert_preview_token_invalid_err(err);
}

#[test]
fn execute_clean_without_preview_rejected() {
    let state = common::make_state();
    let err = commands_impl::execute_clean(&state, "no-such-token".to_string(), None, |_| {})
        .unwrap_err();
    assert_preview_invalid(err);
}

#[test]
fn preview_token_expired_rejected() {
    let s = common::seeded_scan();
    let token = common::preview_scan_token(&s.state);
    s.state.clock.advance_secs(10 * 60 + 1);
    let err = commands_impl::execute_clean(&s.state, token, None, |_| {}).unwrap_err();
    assert_preview_invalid(err);
}

#[test]
fn preview_token_tampered_rejected() {
    let s = common::seeded_scan();
    let _ = common::preview_scan_token(&s.state);
    let err =
        commands_impl::execute_clean(&s.state, "preview-tampered-token".to_string(), None, |_| {})
            .unwrap_err();
    assert_preview_invalid(err);
}

#[test]
fn preview_token_double_use_rejected() {
    let s = common::seeded_scan();
    let token = common::preview_scan_token(&s.state);
    let logs_dir = tempfile::tempdir().unwrap();
    let res = commands_impl::execute_clean(
        &s.state,
        token.clone(),
        Some(logs_dir.path().to_path_buf()),
        |_| {},
    );
    assert!(res.is_ok(), "{res:?}");
    let err =
        commands_impl::execute_clean(&s.state, token, Some(logs_dir.path().to_path_buf()), |_| {})
            .unwrap_err();
    assert_preview_invalid(err);
}

#[test]
fn execute_clean_rejected_when_session_invalidated_by_settings() {
    let s = common::seeded_scan();
    let token = common::preview_scan_token(&s.state);
    let mut settings = s.state.settings.lock().unwrap();
    let mut cfg = settings.get();
    cfg.min_age_hours = 10;
    let _ = settings.update(cfg);
    drop(settings);
    let err = commands_impl::execute_clean(&s.state, token, None, |_| {}).unwrap_err();
    common::assert_session_invalid_err(err);
}

#[test]
fn execute_clean_rejected_when_scan_session_expired() {
    let s = common::seeded_scan();
    let token = common::preview_scan_token(&s.state);
    s.state.clock.advance_secs(15 * 60 + 1);
    let err = commands_impl::execute_clean(&s.state, token, None, |_| {}).unwrap_err();
    common::assert_session_or_preview_invalid(err);
}

fn assert_clean_outcome_counts(resp: &sweeper_contract::ExecuteCleanResponse) {
    assert_eq!(resp.deleted_count, U64String(1));
    assert_eq!(resp.skipped_count, U64String(0));
    assert!(!resp.aborted, "abort_reason={:?}", resp.abort_reason);
}

fn assert_clean_effects(file_path: &std::path::Path, progress_events: usize) {
    assert!(!file_path.exists(), "eligible file should be deleted");
    assert!(progress_events >= 1);
}

#[test]
fn valid_scan_preview_execute_clean_in_sentinel_sandbox_writes_log_and_preserves_canary() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("gate-f-exec").unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let file_path = sandbox
        .write_file("Temp\\deleteme.tmp", b"hello-sweeper")
        .unwrap();
    common::seed_session(&state, &root, &file_path, 1_700_000_000);
    canary.verify().unwrap();
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let logs_base = tempfile::tempdir().unwrap();
    let mut progress_events = 0usize;
    let resp = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| progress_events += 1,
    )
    .unwrap();
    assert_clean_outcome_counts(&resp);
    assert_clean_effects(&file_path, progress_events);
    canary.verify().unwrap();
    let log_id = resp.log_id.clone().expect("log_id must be present");
    let logs_dir = logs_base.path().join("logs");
    let logs = commands_impl::list_logs_in_dir(Some(logs_dir.as_path())).unwrap();
    assert!(logs.iter().any(|l| l.id == log_id), "logs={logs:?}");
    let log = commands_impl::read_log_in_dir(Some(logs_dir.as_path()), &log_id).unwrap();
    common::assert_clean_log_contains_intent_and_outcome(&log);
}

#[test]
fn clean_log_id_is_not_hijacked_by_a_scan_log_written_during_clean() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let file_path = sandbox
        .write_file("Temp\\deleteme-scan-race.tmp", b"hello-sweeper")
        .unwrap();
    common::seed_session(&state, &root, &file_path, 1_700_000_000);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let logs_base = tempfile::tempdir().unwrap();
    let scan_written = AtomicBool::new(false);

    let resp = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_evt| {
            // A concurrent scan writes a newer scan-* log while the Clean is running.
            if !scan_written.swap(true, Ordering::SeqCst) {
                let scan_path = sweeper_core::journal::write_scan_log(
                    logs_base.path(),
                    &serde_json::json!({ "kind": "scan" }),
                )
                .unwrap()
                .path;
                sweeper_testkit::set_last_write_time(
                    &scan_path,
                    sweeper_testkit::filetime_from_unix_secs(2_000_000_000),
                )
                .unwrap();
            }
        },
    )
    .unwrap();

    let log_id = resp.log_id.expect("log_id must be present");
    assert!(
        log_id.starts_with("clean-"),
        "Clean log_id must point at the clean journal, got {log_id}"
    );
}

struct CustomScanCtx {
    state: sweeper_desktop::AppState<sweeper_desktop::clock::FixedClock>,
    _sandbox: Sandbox,
}

fn custom_scan_fixture(
    root: &std::path::Path,
    file_path: &std::path::Path,
) -> (
    sweeper_contract::ScanItemDto,
    sweeper_contract::TargetScanResultDto,
    std::collections::HashMap<String, sweeper_desktop::session::StoredItem>,
) {
    let item = sweeper_contract::ScanItemDto {
        id: "custom-path-1::1".to_string(),
        target_id: "custom-path-1".to_string(),
        path: file_path.to_string_lossy().to_string(),
        size: sweeper_contract::U64String(3),
        mtime_unix_secs: sweeper_contract::U64String(1_700_000_000),
        age_eligible: true,
    };
    let target = sweeper_contract::TargetScanResultDto {
        target_id: "custom-path-1".to_string(),
        target_name: "Custom Path".to_string(),
        tier: sweeper_contract::TierDto::Tier2,
        risk: sweeper_contract::RiskDto::Caution,
        resolved_paths: vec![root.to_string_lossy().to_string()],
        scanned: true,
        status_reason: None,
        total_bytes: sweeper_contract::U64String(3),
        eligible_bytes: sweeper_contract::U64String(3),
        file_count: sweeper_contract::U64String(1),
        eligible_count: sweeper_contract::U64String(1),
        skipped_count: sweeper_contract::U64String(0),
        items: vec![item.clone()],
    };
    let mut items = std::collections::HashMap::new();
    items.insert(
        item.id.clone(),
        sweeper_desktop::session::StoredItem {
            dto: item.clone(),
            target_id: "custom-path-1".to_string(),
            target_name: "Custom Path".to_string(),
            tier: sweeper_core::Tier::Tier2,
            risk: sweeper_core::Risk::Caution,
            age_eligible: true,
            requires_typed_confirmation: true,
        },
    );
    (item, target, items)
}

fn seed_custom_path_scan() -> CustomScanCtx {
    let state = common::make_state();
    let sandbox = Sandbox::new_in(common::sandbox_test_base());
    let root = sandbox.create_dir("CustomRoot").unwrap();
    let file_path = sandbox.write_file("CustomRoot\\old.tmp", b"abc").unwrap();
    let (_item, target, items) = custom_scan_fixture(&root, &file_path);
    let results = sweeper_contract::ScanResultsDto {
        session_id: "scan-custom".to_string(),
        started_unix_secs: sweeper_contract::U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(sweeper_desktop::session::TestScanSessionSeed {
            now: 1_800_000_000,
            settings_generation: state.settings.lock().unwrap().generation(),
            session_id: "scan-custom".to_string(),
            results,
            items,
        });
    CustomScanCtx {
        state,
        _sandbox: sandbox,
    }
}

#[test]
fn typed_confirmation_rejected_for_custom_path_scan_target() {
    let ctx = seed_custom_path_scan();
    let selection = common::SessionItem {
        session_id: "scan-custom",
        item_id: "custom-path-1::1",
    };
    common::preview_rejects_missing_and_wrong_confirmation(&ctx.state, selection);
}

#[test]
fn typed_confirmation_accepted_for_custom_path_scan_target() {
    let ctx = seed_custom_path_scan();
    let selection = common::SessionItem {
        session_id: "scan-custom",
        item_id: "custom-path-1::1",
    };
    let token = common::preview_accepts_delete_confirmation(&ctx.state, selection);
    assert!(!token.is_empty());
}

#[test]
fn session_store_lock_not_held_during_inflight_execute_clean() {
    let state = Arc::new(common::make_state());
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("gate-f-lock-clean").unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let file_path = sandbox
        .write_file("Temp\\deleteme-lock.tmp", b"hello-sweeper")
        .unwrap();
    common::seed_session(&state, &root, &file_path, 1_700_000_000);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let logs_base = tempfile::tempdir().unwrap();
    let try_lock_ok = Arc::new(AtomicBool::new(false));
    let state_cb = Arc::clone(&state);
    let try_lock_ok_cb = Arc::clone(&try_lock_ok);
    let exec = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        move |_evt: sweeper_contract::CleanProgressEvent| {
            if state_cb.sessions.try_lock().is_ok() {
                try_lock_ok_cb.store(true, Ordering::SeqCst);
            }
            std::thread::sleep(Duration::from_millis(1));
        },
    )
    .unwrap();
    assert!(!exec.aborted, "{exec:?}");
    canary.verify().unwrap();
    assert!(try_lock_ok.load(Ordering::SeqCst));
}
