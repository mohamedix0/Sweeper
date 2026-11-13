use std::collections::HashMap;
use std::fs;
use std::path::Path;

use sweeper_contract::{
    ContractError, RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto, U64String,
};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::StoredItem;
use sweeper_desktop::AppState;
use sweeper_testkit::{CanaryTree, Sandbox};

fn make_state() -> AppState<FixedClock> {
    // old mtime for scanned items (>=24h before now)
    AppState::with_clock(FixedClock::new(1_800_000_000))
}

fn seed_session(state: &AppState<FixedClock>, root: &Path, file_path: &Path, mtime_secs: u64) {
    let settings_gen = state.settings.lock().unwrap().generation();
    let root_s = root.to_string_lossy().to_string();
    let path_s = file_path.to_string_lossy().to_string();
    let size = fs::metadata(file_path).map(|m| m.len()).unwrap_or(100);
    let item = ScanItemDto {
        id: "t1::1".to_string(),
        target_id: "user-temp".to_string(),
        path: path_s.clone(),
        size: U64String(size),
        mtime_unix_secs: U64String(mtime_secs),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: "user-temp".to_string(),
        target_name: "User Temp".to_string(),
        tier: TierDto::Tier1,
        risk: RiskDto::Safe,
        resolved_paths: vec![root_s],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(size),
        eligible_bytes: U64String(size),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    };
    let results = ScanResultsDto {
        session_id: "scan-test".to_string(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(
        item.id.clone(),
        StoredItem {
            dto: item,
            target_id: "user-temp".to_string(),
            target_name: "User Temp".to_string(),
            tier: sweeper_knowledge::Tier::Tier1,
            risk: sweeper_knowledge::Risk::Safe,
            age_eligible: true,
            requires_typed_confirmation: false,
        },
    );
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(1_800_000_000, settings_gen, "scan-test", results, items);
}

#[test]
fn unknown_item_ids_rejected() {
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    let err = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["missing".to_string()],
        None,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::UnknownItem { .. }), "{err:?}");
}

#[test]
fn expired_session_after_15_minutes_rejected() {
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);

    state.clock.advance_secs(15 * 60 + 1);
    let err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
    let err = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn invalidated_session_on_settings_generation_change_rejected() {
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    {
        let mut settings = state.settings.lock().unwrap();
        let mut s = settings.get();
        s.min_age_hours = 10;
        let _ = settings.update(s);
    }
    let err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn execute_clean_without_preview_rejected() {
    let state = make_state();
    let err = commands_impl::execute_clean(&state, "no-such-token".to_string(), None, |_| {})
        .unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn preview_token_expired_tampered_double_use_and_session_invalidated_rejected() {
    // Expired token (>10m)
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    state.clock.advance_secs(10 * 60 + 1);
    let err = commands_impl::execute_clean(&state, preview.preview_token.clone(), None, |_| {})
        .unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );

    // Tampered token
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    let _preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let err =
        commands_impl::execute_clean(&state, "preview-tampered-token".to_string(), None, |_| {})
            .unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );

    // Double execute_clean rejected (single-use)
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let logs_dir = tempfile::tempdir().unwrap();
    let res = commands_impl::execute_clean(
        &state,
        preview.preview_token.clone(),
        Some(logs_dir.path().to_path_buf()),
        |_| {},
    );
    assert!(res.is_ok(), "{res:?}");
    let err = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs_dir.path().to_path_buf()),
        |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );

    // Settings-invalidated session rejected by execute_clean
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    {
        let mut settings = state.settings.lock().unwrap();
        let mut s = settings.get();
        s.min_age_hours = 10;
        let _ = settings.update(s);
    }
    let err =
        commands_impl::execute_clean(&state, preview.preview_token, None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );

    // Expired ScanSession (>15m) rejected by execute_clean
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    state.clock.advance_secs(15 * 60 + 1);
    let err =
        commands_impl::execute_clean(&state, preview.preview_token, None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. })
            || matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn valid_scan_preview_execute_clean_in_sentinel_sandbox_writes_log_and_preserves_canary() {
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let canary = CanaryTree::seed_external("gate-f-exec").unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let file_path = sandbox
        .write_file("Temp\\deleteme.tmp", b"hello-sweeper")
        .unwrap();
    assert!(file_path.exists());
    canary.verify().unwrap();

    seed_session(&state, &root, &file_path, 1_700_000_000);
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
        |_| {
            progress_events += 1;
        },
    )
    .unwrap();

    assert_eq!(resp.deleted_count, U64String(1));
    assert_eq!(resp.skipped_count, U64String(0));
    assert!(!resp.aborted, "abort_reason={:?}", resp.abort_reason);
    assert!(
        !file_path.exists(),
        "eligible file should be deleted in sandbox"
    );
    assert!(progress_events >= 1);
    canary.verify().unwrap();

    let log_id = resp.log_id.clone().expect("log_id must be present");
    let logs =
        commands_impl::list_logs_in_dir(Some(logs_base.path().join("logs").as_path())).unwrap();
    assert!(logs.iter().any(|l| l.id == log_id), "logs={logs:?}");
    let log =
        commands_impl::read_log_in_dir(Some(logs_base.path().join("logs").as_path()), &log_id)
            .unwrap();
    assert!(log.content.contains("\"record\":\"intent\""));
    assert!(log.content.contains("\"record\":\"outcome\""));
    assert!(
        log.content.contains("\"status\":\"deleted\"")
            || log.content.contains("\"status\":\"deleted\"")
    );
}

#[test]
fn tauri_config_capabilities_and_command_surface_snapshot() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let conf_path = root.join("tauri.conf.json");
    let cap_path = root.join("capabilities").join("default.json");
    let conf: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&conf_path).unwrap()).unwrap();
    let cap: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&cap_path).unwrap()).unwrap();

    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    assert!(!csp.contains("unsafe-inline"), "csp: {csp}");

    assert_eq!(
        conf["app"]["security"]["freezePrototype"],
        serde_json::json!(true)
    );
    assert_eq!(
        conf["app"]["security"]["pattern"]["use"],
        serde_json::json!("isolation")
    );

    let conf_s = conf.to_string().to_lowercase();
    let cap_s = cap.to_string().to_lowercase();
    assert!(
        !conf_s.contains("plugin-fs")
            && !conf_s.contains("plugin-shell")
            && !conf_s.contains("\"fs\"")
            && !conf_s.contains("\"shell\""),
        "config must not reference fs/shell plugins"
    );
    assert!(
        !cap_s.contains("fs:")
            && !cap_s.contains("shell:")
            && !cap_s.contains("plugin-fs")
            && !cap_s.contains("plugin-shell"),
        "capabilities must not reference fs/shell plugins"
    );

    let mut cmds = cap["commands"]["allow"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    cmds.sort();
    assert_eq!(
        cmds,
        vec![
            "execute_clean".to_string(),
            "explain_item".to_string(),
            "get_scan_results".to_string(),
            "get_settings".to_string(),
            "list_logs".to_string(),
            "preview_clean".to_string(),
            "read_log".to_string(),
            "start_scan".to_string(),
        ]
    );
}
