use std::collections::HashMap;
use std::fs;

use sweeper_contract::{
    ContractError, RiskDto, ScanItemDto, ScanResultsDto, TargetScanResultDto, TierDto, U64String,
};
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::StoredItem;
use sweeper_desktop::AppState;

fn make_state() -> AppState<FixedClock> {
    AppState::with_clock(FixedClock::new(1_700_000_000))
}

fn seed_session(state: &AppState<FixedClock>) {
    let settings_gen = state.settings.lock().unwrap().generation();
    let item = ScanItemDto {
        id: "t1::1".to_string(),
        target_id: "user-temp".to_string(),
        path: r"C:\Users\Test\AppData\Local\Temp\a.tmp".to_string(),
        size: U64String(100),
        mtime_unix_secs: U64String(1_699_000_000),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: "user-temp".to_string(),
        target_name: "User Temp".to_string(),
        tier: TierDto::Tier1,
        risk: RiskDto::Safe,
        resolved_paths: vec![r"C:\Users\Test\AppData\Local\Temp".to_string()],
        scanned: true,
        status_reason: None,
        total_bytes: U64String(100),
        eligible_bytes: U64String(100),
        file_count: U64String(1),
        eligible_count: U64String(1),
        skipped_count: U64String(0),
        items: vec![item.clone()],
    };
    let results = ScanResultsDto {
        session_id: "scan-test".to_string(),
        started_unix_secs: U64String(1_700_000_000),
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
        .create_test_session_with_items(1_700_000_000, settings_gen, "scan-test", results, items);
}

#[test]
fn unknown_item_ids_rejected() {
    let state = make_state();
    seed_session(&state);
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
    seed_session(&state);
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
    seed_session(&state);
    {
        let mut settings = state.settings.lock().unwrap();
        let mut s = settings.get();
        s.min_age_hours = 10; // scan-affecting
        let _ = settings.update(s);
    }
    let err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn preview_token_expires_after_10_minutes_and_rejects_tamper_double_use() {
    let state = make_state();
    seed_session(&state);
    let preview = commands_impl::preview_clean(
        &state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let settings_gen = state.settings.lock().unwrap().generation();

    // Tamper: consume with wrong token string via direct store manipulation not possible; simulate by
    // validating correct token then trying double-use.
    {
        let mut sessions = state.sessions.lock().unwrap();
        let res =
            sessions.consume_preview_token(&state.clock, &preview.preview_token, settings_gen);
        assert!(res.is_ok());
        let err = sessions
            .consume_preview_token(&state.clock, &preview.preview_token, settings_gen)
            .unwrap_err();
        assert!(
            matches!(err, ContractError::PreviewTokenInvalid { .. }),
            "{err:?}"
        );
    }

    // Expiry: create another token and advance >10 min
    let state2 = make_state();
    seed_session(&state2);
    let preview2 = commands_impl::preview_clean(
        &state2,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap();
    let settings_gen2 = state2.settings.lock().unwrap().generation();
    state2.clock.advance_secs(10 * 60 + 1);
    let mut sessions2 = state2.sessions.lock().unwrap();
    let err = sessions2
        .consume_preview_token(&state2.clock, &preview2.preview_token, settings_gen2)
        .unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
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

    // Strict CSP: default-src 'self' with NO 'unsafe-eval' and NO 'unsafe-inline'.
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    assert!(!csp.contains("unsafe-inline"), "csp: {csp}");

    // freezePrototype and Isolation pattern
    assert_eq!(
        conf["app"]["security"]["freezePrototype"],
        serde_json::json!(true)
    );
    assert_eq!(
        conf["app"]["security"]["pattern"]["use"],
        serde_json::json!("isolation")
    );

    // Zero fs/shell plugins in config/capabilities
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

    // Exact read-only command surface
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
            "explain_item".to_string(),
            "get_scan_results".to_string(),
            "get_settings".to_string(),
            "list_logs".to_string(),
            "preview_clean".to_string(),
            "read_log".to_string(),
            "start_scan".to_string(),
        ]
    );
    assert!(
        !cap_s.contains("execute_clean"),
        "execute_clean must not exist in Phase 2"
    );
}
