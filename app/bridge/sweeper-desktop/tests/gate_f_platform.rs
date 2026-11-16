mod common;

use sweeper_desktop::commands_impl;
use sweeper_testkit::Sandbox;

#[test]
fn tauri_config_security_and_bundle_snapshot() {
    let conf = common::read_tauri_json("tauri.conf.json");
    let cap = common::read_tauri_json("cap");
    common::assert_tauri_csp_and_security(&conf);
    assert_eq!(
        conf["app"]["security"]["freezePrototype"],
        serde_json::json!(true)
    );
    assert_eq!(
        conf["app"]["security"]["pattern"]["use"],
        serde_json::json!("isolation")
    );
    common::assert_tauri_no_fs_shell(&conf, &cap);
    common::assert_tauri_bundle_snapshot(&conf);
}

#[test]
fn tauri_capability_command_surface_snapshot() {
    let cap = common::read_tauri_json("cap");
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
            "get_elevation_status".to_string(),
            "get_scan_results".to_string(),
            "get_settings".to_string(),
            "list_logs".to_string(),
            "preview_clean".to_string(),
            "read_log".to_string(),
            "relaunch_elevated".to_string(),
            "start_analyzer".to_string(),
            "start_scan".to_string(),
            "update_settings".to_string(),
            "validate_custom_path".to_string(),
        ]
    );
}

#[test]
fn validate_custom_path_rejects_missing_and_protected_and_accepts_valid_sandbox_path() {
    let state = common::make_state();
    let sandbox_base = common::sandbox_test_base();
    let sandbox = Sandbox::new_in(&sandbox_base);
    let _valid_root = sandbox.create_dir("Allowed").unwrap();
    let nested = sandbox.create_dir("Allowed\\Nested\\Deep").unwrap();
    let ok =
        commands_impl::validate_custom_path(&state, nested.to_string_lossy().to_string()).unwrap();
    assert!(ok.allowed, "expected allowed verdict: {ok:?}");
    let missing = nested.join("does-not-exist");
    let rejected_missing =
        commands_impl::validate_custom_path(&state, missing.to_string_lossy().to_string()).unwrap();
    assert!(
        !rejected_missing.allowed,
        "missing must be rejected: {rejected_missing:?}"
    );
    let protected =
        commands_impl::validate_custom_path(&state, r"C:\Windows\System32".to_string()).unwrap();
    assert!(
        !protected.allowed,
        "protected must be rejected: {protected:?}"
    );
}

#[test]
fn tampered_settings_json_recovers_to_safe_defaults_with_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let settings_path = tmp.path().join("settings.json");
    std::fs::write(
        &settings_path,
        r#"{
  "enable_tier1": "nope",
  "enable_tier3": true,
  "min_age_hours": 999999,
  "allow_delete_unknown": "bad",
  "custom_paths": ["C:\\Windows\\System32", "not\\absolute", "C:\\missing\\nope"],
  "stale_project_days": 0,
  "protected_paths": ["C:\\Windows"]
}"#,
    )
    .unwrap();
    let store = sweeper_desktop::settings_store::SettingsStore::with_path(settings_path);
    let dto = store.get();
    let core = store.core();
    assert!(dto.enable_tier1, "invalid bool should fall back");
    assert_eq!(dto.min_age_hours, 24, "out-of-bounds min_age_hours");
    assert_eq!(
        dto.stale_project_days, 30,
        "out-of-bounds stale_project_days"
    );
    assert!(!dto.allow_delete_unknown, "invalid flag should fall back");
    assert!(
        dto.custom_paths.is_empty(),
        "unsafe paths must be dropped: {:?}",
        dto.custom_paths
    );
    assert!(
        !store.warnings().is_empty(),
        "tampered settings should emit warnings"
    );
    assert!(core.enable_tier3, "valid tier3 opt-in should be preserved");
}

#[test]
fn update_settings_increments_generation_and_invalidates_active_sessions() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    common::seed_session(&state, &root, &f, 1_700_000_000);
    common::insert_analyzer_session(
        &state,
        "analyzer-test-session",
        common::base_analyzer_node(common::AnalyzerNodeSeed {
            id: "an-root",
            path: &root.to_string_lossy(),
            name: "Temp",
            is_dir: true,
        }),
        24,
    );
    let mut s = commands_impl::get_settings(&state).unwrap();
    s.stale_project_days = 45;
    let updated = commands_impl::update_settings(&state, s).unwrap();
    assert_eq!(updated.stale_project_days, 45);
    let scan_err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    common::assert_session_invalid_err(scan_err);
    let preview_err = common::preview_with_delete_confirmation(
        &state,
        "analyzer-test-session",
        "an-root".to_string(),
    )
    .unwrap_err();
    common::assert_session_invalid_err(preview_err);
}

#[test]
fn tier3_default_off_and_opt_in_reflected_in_settings() {
    let state = common::make_state();
    let got = commands_impl::get_settings(&state).unwrap();
    assert!(!got.enable_tier3, "Tier 3 must default to off");
    let mut s = got.clone();
    s.enable_tier3 = true;
    let updated = commands_impl::update_settings(&state, s).unwrap();
    assert!(updated.enable_tier3, "Tier 3 opt-in must persist");
}

#[derive(Clone, Copy)]
struct ElevationStatusCase {
    elevated: bool,
    expected_calls: usize,
}

fn assert_elevation_status(case: ElevationStatusCase) {
    let ctx = common::make_elevation_test_state(
        case.elevated,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    );
    let got = commands_impl::get_elevation_status(&ctx.state).unwrap();
    assert_eq!(got.is_elevated, case.elevated);
    assert_eq!(
        ctx.provider
            .status_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        case.expected_calls
    );
}

#[test]
fn get_elevation_status_reports_provider_state() {
    assert_elevation_status(ElevationStatusCase {
        elevated: false,
        expected_calls: 1,
    });
    assert_elevation_status(ElevationStatusCase {
        elevated: true,
        expected_calls: 1,
    });
}

#[derive(Clone, Copy, Debug)]
struct RelaunchExpectation {
    calls: usize,
    exited: bool,
    exit_code: Option<i32>,
    held: bool,
}

fn assert_relaunch_state(ctx: &common::ElevationTestState, expected: RelaunchExpectation) {
    assert_eq!(
        ctx.provider
            .relaunch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        expected.calls
    );
    assert_eq!(
        ctx.exited.load(std::sync::atomic::Ordering::SeqCst),
        expected.exited
    );
    assert_eq!(*ctx.exit_code.lock().unwrap(), expected.exit_code);
    assert_eq!(ctx.si.is_held(), expected.held);
}

#[derive(Clone, Copy)]
enum RelaunchCase {
    AlreadyElevated,
    Cancelled,
    Launched,
}

fn run_relaunch_case(case: RelaunchCase) {
    match case {
        RelaunchCase::AlreadyElevated => {
            let ctx = common::make_elevation_test_state(
                true,
                Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
            );
            let res = commands_impl::relaunch_elevated(&ctx.state).unwrap();
            assert!(!res.launched);
            assert_relaunch_state(
                &ctx,
                RelaunchExpectation {
                    calls: 0,
                    exited: false,
                    exit_code: None,
                    held: true,
                },
            );
        }
        RelaunchCase::Cancelled => {
            let ctx = common::make_elevation_test_state(
                false,
                Ok(sweeper_core::elevation::RelaunchOutcome::CancelledByUser),
            );
            let err = commands_impl::relaunch_elevated(&ctx.state).unwrap_err();
            common::assert_relaunch_cancel_preserved(&ctx, err);
        }
        RelaunchCase::Launched => {
            let ctx = common::make_elevation_test_state(
                false,
                Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
            );
            let res = commands_impl::relaunch_elevated(&ctx.state).unwrap();
            assert!(res.launched);
            assert_relaunch_state(
                &ctx,
                RelaunchExpectation {
                    calls: 1,
                    exited: true,
                    exit_code: Some(0),
                    held: false,
                },
            );
        }
    }
}

#[test]
fn relaunch_elevated_when_already_elevated_returns_not_launched_without_side_effects() {
    run_relaunch_case(RelaunchCase::AlreadyElevated);
}

#[test]
fn relaunch_elevated_uac_cancel_returns_error_preserves_guard_and_does_not_exit() {
    run_relaunch_case(RelaunchCase::Cancelled);
}

#[test]
fn relaunch_elevated_success_releases_guard_invokes_launcher_returns_launched_and_triggers_exit() {
    run_relaunch_case(RelaunchCase::Launched);
}

#[test]
fn single_instance_guard_cross_thread_handoff_via_app_state() {
    let si = common::make_single_instance_state_with_unique_mutex("SweeperGateF-cross-thread");
    let si_worker = std::sync::Arc::clone(&si);
    let handle = std::thread::spawn(move || {
        let mut guard = si_worker.take().expect("take guard");
        guard.release_for_handoff().expect("release");
        guard.reacquire(1_000).expect("reacquire");
        si_worker.put(guard);
    });
    handle.join().expect("join");
    assert!(si.is_held());
}
