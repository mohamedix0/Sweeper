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

#[test]
fn single_instance_guard_cross_thread_handoff_via_app_state() {
    use std::sync::Arc;

    fn unique_test_mutex_name(suffix: &str) -> String {
        static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!(r"Local\SweeperGateF-{pid}-{nanos}-{n}-{suffix}")
    }

    let name = unique_test_mutex_name("app-state-cross-thread");
    let guard = sweeper_core::elevation::SingleInstanceGuard::acquire_named(&name, 1_000)
        .expect("acquire named single-instance guard");
    let state = Arc::new(AppState::new_default().with_single_instance_guard(guard));
    assert!(
        state.single_instance.is_held(),
        "startup guard should be held in AppState"
    );

    let state_for_worker = Arc::clone(&state);
    let contender_name = name.clone();
    let handle = std::thread::spawn(move || {
        let mut guard = state_for_worker
            .single_instance
            .take()
            .expect("take guard from AppState on worker thread");
        assert!(guard.is_held());

        guard
            .release_for_handoff()
            .expect("release_for_handoff must succeed on worker thread");

        let contender = std::thread::spawn(move || {
            sweeper_core::elevation::SingleInstanceGuard::acquire_named(&contender_name, 1_000)
        })
        .join()
        .expect("contender join")
        .expect("contender acquires released mutex");
        drop(contender);

        guard
            .reacquire(1_000)
            .expect("reacquire must succeed on worker thread");
        state_for_worker.single_instance.put(guard);
    });
    handle.join().expect("worker thread join");

    assert!(
        state.single_instance.is_held(),
        "guard should be held again in AppState after cross-thread reacquire"
    );
}

fn make_state() -> AppState<FixedClock> {
    // old mtime for scanned items (>=24h before now)
    AppState::with_clock(FixedClock::new(1_800_000_000))
}

fn state_with_temp_settings<F>(mutate: F) -> AppState<FixedClock>
where
    F: FnOnce(&mut sweeper_contract::SettingsDto),
{
    let state = make_state();
    let mut settings = state.settings.lock().unwrap().get();
    mutate(&mut settings);
    state.settings.lock().unwrap().update(settings);
    state
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
            tier: sweeper_core::Tier::Tier1,
            risk: sweeper_core::Risk::Safe,
            age_eligible: true,
            requires_typed_confirmation: false,
        },
    );
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(sweeper_desktop::session::TestScanSessionSeed {
            now: 1_800_000_000,
            settings_generation: settings_gen,
            session_id: "scan-test".to_string(),
            results,
            items,
        });
}

fn sandbox_test_base() -> std::path::PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target-fallback")
        })
        .join("sweeper-test-sandboxes");
    let _ = std::fs::create_dir_all(&base);
    let normalized = match std::fs::canonicalize(&base) {
        Ok(c) => {
            let s = c.to_string_lossy();
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                std::path::PathBuf::from(format!(r"\\{}", rest))
            } else if let Some(rest) = s.strip_prefix(r"\\?\") {
                std::path::PathBuf::from(rest.to_string())
            } else {
                c
            }
        }
        Err(_) => base.components().collect::<std::path::PathBuf>(),
    };
    debug_assert!(
        !normalized
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "sandbox base must not contain '..' components: {}",
        normalized.display()
    );
    normalized
}

#[test]
fn unknown_item_ids_rejected() {
    let state = make_state();
    let sandbox = Sandbox::new();
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
    let sandbox = Sandbox::new();
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
    let sandbox = Sandbox::new();
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

struct SeededScan {
    state: AppState<FixedClock>,
    _sandbox: Sandbox,
}

fn seeded_scan() -> SeededScan {
    let state = make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    SeededScan {
        state,
        _sandbox: sandbox,
    }
}

fn preview_scan_token(state: &AppState<FixedClock>) -> String {
    commands_impl::preview_clean(
        state,
        "scan-test".to_string(),
        vec!["t1::1".to_string()],
        None,
    )
    .unwrap()
    .preview_token
}

#[test]
fn preview_token_expired_rejected() {
    let s = seeded_scan();
    let token = preview_scan_token(&s.state);
    s.state.clock.advance_secs(10 * 60 + 1);
    let err = commands_impl::execute_clean(&s.state, token, None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn preview_token_tampered_rejected() {
    let s = seeded_scan();
    let _ = preview_scan_token(&s.state);
    let err =
        commands_impl::execute_clean(&s.state, "preview-tampered-token".to_string(), None, |_| {})
            .unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn preview_token_double_use_rejected() {
    let s = seeded_scan();
    let token = preview_scan_token(&s.state);
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
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn execute_clean_rejected_when_session_invalidated_by_settings() {
    let s = seeded_scan();
    let token = preview_scan_token(&s.state);
    let mut settings = s.state.settings.lock().unwrap();
    let mut cfg = settings.get();
    cfg.min_age_hours = 10;
    let _ = settings.update(cfg);
    drop(settings);
    let err = commands_impl::execute_clean(&s.state, token, None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn execute_clean_rejected_when_scan_session_expired() {
    let s = seeded_scan();
    let token = preview_scan_token(&s.state);
    s.state.clock.advance_secs(15 * 60 + 1);
    let err = commands_impl::execute_clean(&s.state, token, None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. })
            || matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn valid_scan_preview_execute_clean_in_sentinel_sandbox_writes_log_and_preserves_canary() {
    let state = make_state();
    let sandbox = Sandbox::new();
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

fn read_tauri_json(file: &str) -> serde_json::Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = match file {
        "cap" => root.join("capabilities").join("default.json"),
        _ => root.join(file),
    };
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_tauri_csp_and_security(conf: &serde_json::Value) {
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    assert!(!csp.contains("unsafe-inline"), "csp: {csp}");
}

fn assert_tauri_no_fs_shell(conf: &serde_json::Value, cap: &serde_json::Value) {
    let conf_s = conf.to_string().to_lowercase();
    let cap_s = cap.to_string().to_lowercase();
    assert!(
        !conf_s.contains("plugin-fs") && !conf_s.contains("plugin-shell"),
        "config plugins"
    );
    assert!(
        !cap_s.contains("fs:") && !cap_s.contains("shell:"),
        "capability plugins"
    );
}

#[test]
fn tauri_config_security_and_bundle_snapshot() {
    let conf = read_tauri_json("tauri.conf.json");
    let cap = read_tauri_json("cap");
    assert_tauri_csp_and_security(&conf);
    assert_eq!(
        conf["app"]["security"]["freezePrototype"],
        serde_json::json!(true)
    );
    assert_eq!(
        conf["app"]["security"]["pattern"]["use"],
        serde_json::json!("isolation")
    );
    assert_tauri_no_fs_shell(&conf, &cap);

    assert_eq!(conf["bundle"]["active"], serde_json::json!(true));
    let targets = conf["bundle"]["targets"].as_array().unwrap();
    assert!(
        targets.iter().any(|t| t.as_str() == Some("nsis")),
        "{targets:?}"
    );
    assert!(!conf["bundle"]["icon"]
        .as_array()
        .unwrap_or(&vec![])
        .is_empty());
}

#[test]
fn tauri_capability_command_surface_snapshot() {
    let cap = read_tauri_json("cap");
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

fn find_node_by_path_suffix<'a>(
    node: &'a sweeper_contract::AnalyzerNodeDto,
    suffix: &str,
) -> Option<&'a sweeper_contract::AnalyzerNodeDto> {
    if node
        .path
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        return Some(node);
    }
    for c in &node.children {
        if let Some(found) = find_node_by_path_suffix(c, suffix) {
            return Some(found);
        }
    }
    None
}

#[allow(dead_code)]
fn find_node_by_id<'a>(
    node: &'a sweeper_contract::AnalyzerNodeDto,
    id: &str,
) -> Option<&'a sweeper_contract::AnalyzerNodeDto> {
    if node.id == id {
        return Some(node);
    }
    for c in &node.children {
        if let Some(found) = find_node_by_id(c, id) {
            return Some(found);
        }
    }
    None
}

#[test]
fn start_analyzer_on_sandbox_returns_tree_marks_non_selectable_and_populates_session() {
    let state = make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("A").unwrap();
    let _f1 = sandbox.write_file("A\\file1.txt", b"hello").unwrap();
    let _f2 = sandbox.write_file("A\\sub\\file2.bin", [0u8; 10]).unwrap();
    let _vhdx = sandbox.write_file("A\\disk.vhdx", b"vhdx-bytes").unwrap();
    let _git = sandbox.create_dir("A\\.git").unwrap();
    let _gitf = sandbox.write_file("A\\.git\\config", b"[core]\n").unwrap();
    let outside = sandbox.create_dir("outside").unwrap();
    let link = sandbox.guarded_path("A\\linkdir");
    let _ = sweeper_testkit::create_junction(&link, &outside);

    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();

    assert!(!resp.session_id.is_empty());
    assert!(resp.total_bytes.get() >= 15);
    assert!(resp.total_items.get() >= 2);

    let vhdx = find_node_by_path_suffix(&resp.root, "\\disk.vhdx").expect("vhdx node");
    assert!(vhdx.display_only, "vhdx must be display_only");
    assert!(!vhdx.selectable, "vhdx must not be selectable");

    let git = find_node_by_path_suffix(&resp.root, "\\.git").expect(".git node");
    assert_eq!(git.risk, RiskDto::Protected);
    assert!(!git.selectable, ".git must not be selectable");

    if let Some(link_node) = find_node_by_path_suffix(&resp.root, "\\linkdir") {
        assert!(link_node.is_link_point);
        assert!(!link_node.selectable);
    }

    // Session populated and explainable
    let explained = commands_impl::explain_item(&state, resp.root.id.clone()).unwrap();
    assert!(!explained.description.is_empty());
}

fn assert_unknown_file_not_selectable(root_node: &sweeper_contract::AnalyzerNodeDto) {
    let file_node = find_node_by_path_suffix(root_node, "\\StaleCache\\old.txt")
        .expect("old.txt node in analyzer tree");
    assert!(!file_node.selectable, "reason={:?}", file_node.reason);
    assert_eq!(
        file_node.reason.as_deref(),
        Some("Unknown files are not selectable; only eligible directories can be selected"),
        "{:?}",
        file_node.reason
    );
}

fn assert_stale_cache_dir_selectable(
    root_node: &sweeper_contract::AnalyzerNodeDto,
) -> sweeper_contract::AnalyzerNodeDto {
    let node = find_node_by_path_suffix(root_node, "\\StaleCache").expect("StaleCache node");
    assert!(node.is_dir, "StaleCache must be a directory node");
    assert!(node.selectable, "reason={:?}", node.reason);
    assert!(!node.display_only, "StaleCache must not be display_only");

    let treated_as_custom = node.target_id.is_none() || node.risk == RiskDto::Unknown;
    assert!(treated_as_custom, "{node:?}");
    node.clone()
}

fn exec_analyzer_preview(
    state: &AppState<FixedClock>,
    session_id: &str,
    node_id: String,
) -> sweeper_contract::ExecuteCleanResponse {
    let preview = commands_impl::preview_clean(
        state,
        session_id.to_string(),
        vec![node_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(preview.total_selected_count.get() >= 1);
    let logs_base = tempfile::tempdir().unwrap();
    commands_impl::execute_clean(
        state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .unwrap()
}

#[test]
fn analyzer_eligible_node_preview_and_execute_clean_in_sentinel_sandbox() {
    let state = state_with_temp_settings(|s| s.allow_delete_unknown = true);
    let canary = CanaryTree::seed_external("gate-f-analyzer-exec").unwrap();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("CleanMe").unwrap();
    let stale_cache_dir = sandbox.create_dir("CleanMe\\StaleCache").unwrap();
    let file_path = sandbox
        .write_file("CleanMe\\StaleCache\\old.txt", b"delete-me")
        .unwrap();
    assert!(file_path.exists());
    canary.verify().unwrap();

    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();

    assert_unknown_file_not_selectable(&resp.root);
    let stale_cache_node = assert_stale_cache_dir_selectable(&resp.root);
    let exec = exec_analyzer_preview(&state, &resp.session_id, stale_cache_node.id);

    assert!(!exec.aborted, "abort_reason={:?}", exec.abort_reason);
    assert!(exec.deleted_count.get() >= 1, "expected deleted: {exec:?}");
    canary.verify().unwrap();
    assert!(!sandbox
        .guarded_path("CleanMe\\StaleCache\\old.txt")
        .exists());
    assert!(
        stale_cache_dir.exists(),
        "selected directory root must be preserved"
    );
    assert!(root.exists(), "analyzer root directory must be preserved");
}

struct AnalyzerRejectCtx {
    state: AppState<FixedClock>,
    _sandbox: Sandbox,
    session_id: String,
    root_id: String,
    vhdx_id: Option<String>,
    git_id: Option<String>,
    link_id: Option<String>,
}

fn analyzer_reject_ctx() -> AnalyzerRejectCtx {
    let state = make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("R").unwrap();
    let _ = sandbox.write_file("R\\ok.txt", b"ok").unwrap();
    let _ = sandbox.write_file("R\\disk.vhdx", b"x").unwrap();
    let _ = sandbox.create_dir("R\\.git").unwrap();
    let outside = sandbox.create_dir("outside").unwrap();
    let link = sandbox.guarded_path("R\\linkdir");
    let _ = sweeper_testkit::create_junction(&link, &outside);
    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();
    AnalyzerRejectCtx {
        session_id: resp.session_id.clone(),
        root_id: resp.root.id.clone(),
        vhdx_id: find_node_by_path_suffix(&resp.root, "\\disk.vhdx").map(|n| n.id.clone()),
        git_id: find_node_by_path_suffix(&resp.root, "\\.git").map(|n| n.id.clone()),
        link_id: find_node_by_path_suffix(&resp.root, "\\linkdir").map(|n| n.id.clone()),
        state,
        _sandbox: sandbox,
    }
}

#[test]
fn analyzer_preview_rejects_invalid_nodes() {
    let ctx = analyzer_reject_ctx();
    if let Some(id) = &ctx.vhdx_id {
        let err = commands_impl::preview_clean(
            &ctx.state,
            ctx.session_id.clone(),
            vec![id.clone()],
            None,
        )
        .unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. })
                || matches!(err, ContractError::UnknownItem { .. }),
            "{err:?}"
        );
    }
    if let Some(id) = &ctx.git_id {
        let err = commands_impl::preview_clean(
            &ctx.state,
            ctx.session_id.clone(),
            vec![id.clone()],
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
    }
    if let Some(id) = &ctx.link_id {
        let err = commands_impl::preview_clean(
            &ctx.state,
            ctx.session_id.clone(),
            vec![id.clone()],
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
    }
}

#[test]
fn analyzer_session_id_not_accepted_as_preview_token() {
    let ctx = analyzer_reject_ctx();
    let err = commands_impl::execute_clean(&ctx.state, ctx.session_id, None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn analyzer_session_expired_rejected() {
    let ctx = analyzer_reject_ctx();
    ctx.state.clock.advance_secs(15 * 60 + 1);
    let err = commands_impl::preview_clean(
        &ctx.state,
        ctx.session_id,
        vec![ctx.root_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".into()),
        }),
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

#[test]
fn analyzer_session_invalidated_by_settings_change_rejected() {
    let state = make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("R2").unwrap();
    let _f = sandbox.write_file("R2\\ok.txt", b"ok").unwrap();
    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();
    {
        let mut settings = state.settings.lock().unwrap();
        let mut s = settings.get();
        s.min_age_hours = 10;
        let _ = settings.update(s);
    }
    let err = commands_impl::preview_clean(
        &state,
        resp.session_id,
        vec![resp.root.id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".into()),
        }),
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::SessionInvalid { .. }),
        "{err:?}"
    );
}

fn seed_unknown_dir_session(state: &AppState<FixedClock>) -> String {
    let settings_gen = state.settings.lock().unwrap().generation();
    let unknown_dir = sweeper_core::analyzer::AnalyzerNode {
        id: "node-unknown-dir".to_string(),
        path: r"C:\Users\Public\SweeperUnknownDir".to_string(),
        name: "SweeperUnknownDir".to_string(),
        is_dir: true,
        size_bytes: 123,
        item_count: 1,
        risk: sweeper_core::Risk::Unknown,
        description: "Unknown directory".to_string(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: false,
        selection_reason: Some(
            "Unknown items are not selectable by default in Phase 4".to_string(),
        ),
        children: Vec::new(),
    };
    let root = sweeper_core::analyzer::AnalyzerNode {
        id: "node-root".to_string(),
        path: r"C:\Users\Public".to_string(),
        name: "Public".to_string(),
        is_dir: true,
        size_bytes: 123,
        item_count: 2,
        risk: sweeper_core::Risk::Protected,
        description: "Protected container/root context".to_string(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: false,
        selection_reason: Some("Protected path is not selectable".to_string()),
        children: vec![unknown_dir],
    };
    state.sessions.lock().unwrap().insert_test_analyzer_session(
        sweeper_desktop::session::AnalyzerSessionInput {
            id: "analyzer-unknown-test".to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen,
            result: sweeper_core::analyzer::AnalyzerResult {
                root,
                progress: sweeper_core::analyzer::AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours: 24,
        },
    );
    "node-unknown-dir".to_string()
}

#[test]
fn analyzer_unknown_node_not_selectable_and_rejected_when_unknown_deletions_disabled() {
    let state = make_state();
    assert!(!state.settings.lock().unwrap().get().allow_delete_unknown);
    let node_id = seed_unknown_dir_session(&state);
    let err = commands_impl::preview_clean(
        &state,
        "analyzer-unknown-test".to_string(),
        vec![node_id],
        None,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}

#[test]
fn validate_custom_path_rejects_missing_and_protected_and_accepts_valid_sandbox_path() {
    let state = make_state();
    let sandbox_base = sandbox_test_base();
    let sandbox = sweeper_testkit::Sandbox::new_in(&sandbox_base);
    let valid_root = sandbox.create_dir("Allowed").unwrap();
    let nested = sandbox.create_dir("Allowed\\Nested\\Deep").unwrap();

    let ok =
        commands_impl::validate_custom_path(&state, nested.to_string_lossy().to_string()).unwrap();
    assert!(
        ok.allowed,
        "expected allowed verdict for sandbox path: {ok:?}"
    );

    let missing = nested.join("does-not-exist");
    let rejected_missing =
        commands_impl::validate_custom_path(&state, missing.to_string_lossy().to_string()).unwrap();
    assert!(
        !rejected_missing.allowed,
        "missing path must be rejected: {rejected_missing:?}"
    );

    let protected =
        commands_impl::validate_custom_path(&state, r"C:\Windows\System32".to_string()).unwrap();
    assert!(
        !protected.allowed,
        "protected path must be rejected: {protected:?}"
    );

    drop(valid_root);
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
    assert_eq!(
        dto.min_age_hours, 24,
        "out-of-bounds min_age_hours should reset"
    );
    assert_eq!(
        dto.stale_project_days, 30,
        "out-of-bounds stale_project_days should reset"
    );
    assert!(
        !dto.allow_delete_unknown,
        "invalid allow_delete_unknown should fall back"
    );
    assert!(
        dto.custom_paths.is_empty(),
        "unsafe/missing custom paths must be dropped: {:?}",
        dto.custom_paths
    );
    assert!(
        !store.warnings().is_empty(),
        "tampered settings should emit sanitization warnings"
    );
    assert!(
        core.enable_tier3,
        "valid explicit tier3 opt-in should be preserved when field is valid"
    );
}

fn seed_simple_analyzer_session(state: &AppState<FixedClock>, root: &Path) {
    let settings_gen = state.settings.lock().unwrap().generation();
    state.sessions.lock().unwrap().insert_test_analyzer_session(
        sweeper_desktop::session::AnalyzerSessionInput {
            id: "analyzer-test-session".to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen,
            result: sweeper_core::analyzer::AnalyzerResult {
                root: sweeper_core::analyzer::AnalyzerNode {
                    id: "an-root".to_string(),
                    path: root.to_string_lossy().to_string(),
                    name: "Temp".to_string(),
                    is_dir: true,
                    size_bytes: 3,
                    item_count: 1,
                    risk: sweeper_core::Risk::Safe,
                    description: "d".to_string(),
                    tier: None,
                    target_id: None,
                    is_link_point: false,
                    is_cloud_placeholder: false,
                    display_only: false,
                    selectable: false,
                    selection_reason: None,
                    children: vec![],
                },
                progress: sweeper_core::analyzer::AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours: 24,
        },
    );
}

#[test]
fn update_settings_increments_generation_and_invalidates_active_sessions() {
    let state = make_state();
    let sandbox = sweeper_testkit::Sandbox::new();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);
    seed_simple_analyzer_session(&state, &root);

    let mut s = commands_impl::get_settings(&state).unwrap();
    s.stale_project_days = 45;
    let updated = commands_impl::update_settings(&state, s).unwrap();
    assert_eq!(updated.stale_project_days, 45);

    let scan_err = commands_impl::get_scan_results(&state, "scan-test".to_string()).unwrap_err();
    assert!(
        matches!(scan_err, ContractError::SessionInvalid { .. }),
        "{scan_err:?}"
    );
    let preview_err = commands_impl::preview_clean(
        &state,
        "analyzer-test-session".to_string(),
        vec!["an-root".to_string()],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap_err();
    assert!(
        matches!(preview_err, ContractError::SessionInvalid { .. }),
        "{preview_err:?}"
    );
}

#[test]
fn tier3_default_off_and_opt_in_reflected_in_settings() {
    let state = make_state();
    let got = commands_impl::get_settings(&state).unwrap();
    assert!(!got.enable_tier3, "Tier 3 must default to off");
    let mut s = got.clone();
    s.enable_tier3 = true;
    let updated = commands_impl::update_settings(&state, s).unwrap();
    assert!(
        updated.enable_tier3,
        "Tier 3 opt-in must persist when enabled"
    );
}

struct CustomScanCtx {
    state: AppState<FixedClock>,
    item_id: String,
    _sandbox: sweeper_testkit::Sandbox,
}

fn build_custom_scan_seed(
    root: &Path,
    file_path: &Path,
) -> (ScanResultsDto, HashMap<String, StoredItem>, String) {
    let size = fs::metadata(file_path).map(|m| m.len()).unwrap_or(3);
    let item = ScanItemDto {
        id: "custom-path-1::1".to_string(),
        target_id: "custom-path-1".to_string(),
        path: file_path.to_string_lossy().to_string(),
        size: U64String(size),
        mtime_unix_secs: U64String(1_700_000_000),
        age_eligible: true,
    };
    let target = TargetScanResultDto {
        target_id: "custom-path-1".to_string(),
        target_name: "Custom Path".to_string(),
        tier: TierDto::Tier2,
        risk: RiskDto::Caution,
        resolved_paths: vec![root.to_string_lossy().to_string()],
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
        session_id: "scan-custom".to_string(),
        started_unix_secs: U64String(1_800_000_000),
        min_age_hours: 24,
        targets: vec![target],
    };
    let mut items = HashMap::new();
    items.insert(
        item.id.clone(),
        StoredItem {
            dto: item.clone(),
            target_id: "custom-path-1".to_string(),
            target_name: "Custom Path".to_string(),
            tier: sweeper_core::Tier::Tier2,
            risk: sweeper_core::Risk::Caution,
            age_eligible: true,
            requires_typed_confirmation: true,
        },
    );
    (results, items, item.id)
}

fn seed_custom_scan_session() -> CustomScanCtx {
    let state = make_state();
    let settings_gen = state.settings.lock().unwrap().generation();
    let sandbox = sweeper_testkit::Sandbox::new_in(sandbox_test_base());
    let root = sandbox.create_dir("CustomRoot").unwrap();
    let file_path = sandbox.write_file("CustomRoot\\old.tmp", b"abc").unwrap();
    let (results, items, item_id) = build_custom_scan_seed(&root, &file_path);
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(sweeper_desktop::session::TestScanSessionSeed {
            now: 1_800_000_000,
            settings_generation: settings_gen,
            session_id: "scan-custom".to_string(),
            results,
            items,
        });
    CustomScanCtx {
        state,
        item_id,
        _sandbox: sandbox,
    }
}

#[test]
fn typed_confirmation_rejected_for_custom_path_scan_target() {
    let ctx = seed_custom_scan_session();
    let err = commands_impl::preview_clean(
        &ctx.state,
        "scan-custom".to_string(),
        vec![ctx.item_id.clone()],
        None,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
    let err = commands_impl::preview_clean(
        &ctx.state,
        "scan-custom".to_string(),
        vec![ctx.item_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("WRONG".to_string()),
        }),
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}

#[test]
fn typed_confirmation_accepted_for_custom_path_scan_target() {
    let ctx = seed_custom_scan_session();
    let ok = commands_impl::preview_clean(
        &ctx.state,
        "scan-custom".to_string(),
        vec![ctx.item_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(!ok.preview_token.is_empty());
}

struct UnknownAnalyzerCtx {
    state: AppState<FixedClock>,
    node_id: String,
    _sandbox: sweeper_testkit::Sandbox,
}

fn seed_unknown_analyzer_session() -> UnknownAnalyzerCtx {
    let state = state_with_temp_settings(|s| s.allow_delete_unknown = true);
    let settings_gen = state.settings.lock().unwrap().generation();
    let sandbox = sweeper_testkit::Sandbox::new_in(sandbox_test_base());
    let uroot = sandbox.create_dir("UnknownRoot").unwrap();
    let unknown_node = sweeper_core::analyzer::AnalyzerNode {
        id: "node-unknown-custom".to_string(),
        path: uroot.to_string_lossy().to_string(),
        name: "UnknownRoot".to_string(),
        is_dir: true,
        size_bytes: 0,
        item_count: 0,
        risk: sweeper_core::Risk::Unknown,
        description: "Unknown".to_string(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: true,
        selection_reason: None,
        children: vec![],
    };
    let node_id = unknown_node.id.clone();
    let tree_root = sweeper_core::analyzer::AnalyzerNode {
        id: "node-tree-root-u".to_string(),
        path: uroot.to_string_lossy().to_string(),
        name: "Root".to_string(),
        is_dir: true,
        size_bytes: 0,
        item_count: 1,
        risk: sweeper_core::Risk::Safe,
        description: "root".to_string(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: false,
        selection_reason: None,
        children: vec![unknown_node],
    };
    state.sessions.lock().unwrap().insert_test_analyzer_session(
        sweeper_desktop::session::AnalyzerSessionInput {
            id: "analyzer-unknown-ok".to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen,
            result: sweeper_core::analyzer::AnalyzerResult {
                root: tree_root,
                progress: sweeper_core::analyzer::AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours: 24,
        },
    );
    UnknownAnalyzerCtx {
        state,
        node_id,
        _sandbox: sandbox,
    }
}

#[test]
fn typed_confirmation_rejected_for_unknown_analyzer_items() {
    let ctx = seed_unknown_analyzer_session();
    let err = commands_impl::preview_clean(
        &ctx.state,
        "analyzer-unknown-ok".to_string(),
        vec![ctx.node_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("wrong".to_string()),
        }),
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}

#[test]
fn typed_confirmation_accepted_for_unknown_analyzer_items() {
    let ctx = seed_unknown_analyzer_session();
    let ok = commands_impl::preview_clean(
        &ctx.state,
        "analyzer-unknown-ok".to_string(),
        vec![ctx.node_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(!ok.preview_token.is_empty());
}

struct CustomRootCtx {
    state: AppState<FixedClock>,
    sandbox: Sandbox,
    canary: CanaryTree,
    root_path: std::path::PathBuf,
    inside: std::path::PathBuf,
    nested: std::path::PathBuf,
    node_id: String,
}

fn setup_custom_root_fs(
    sandbox: &Sandbox,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root_path = sandbox.create_dir("CustomCleanRoot").unwrap();
    let inside = sandbox
        .write_file("CustomCleanRoot\\old.tmp", b"bye")
        .unwrap();
    let _ = sandbox.create_dir("CustomCleanRoot\\nested").unwrap();
    let nested = sandbox
        .write_file("CustomCleanRoot\\nested\\x.tmp", b"x")
        .unwrap();
    assert!(root_path.exists() && inside.exists() && nested.exists());
    let verdict = sweeper_core::policy::validate_custom_path(&root_path.to_string_lossy());
    assert!(
        verdict.allowed,
        "{} ({})",
        root_path.display(),
        verdict.reason
    );
    (root_path, inside, nested)
}

fn insert_custom_root_session(state: &AppState<FixedClock>, root_path: &Path) -> String {
    let settings_gen = state.settings.lock().unwrap().generation();
    let root_str = root_path.to_string_lossy().to_string();
    let custom_root_node = sweeper_core::analyzer::AnalyzerNode {
        id: "node-custom-root".to_string(),
        path: root_str.clone(),
        name: "CustomCleanRoot".to_string(),
        is_dir: true,
        size_bytes: 4,
        item_count: 2,
        risk: sweeper_core::Risk::Safe,
        description: "Custom path root candidate".to_string(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: true,
        selection_reason: None,
        children: Vec::new(),
    };
    let node_id = custom_root_node.id.clone();
    let tree_root = sweeper_core::analyzer::AnalyzerNode {
        id: "node-tree-root".to_string(),
        path: root_str,
        name: "CustomCleanRoot".to_string(),
        is_dir: true,
        size_bytes: 4,
        item_count: 3,
        risk: sweeper_core::Risk::Safe,
        description: "Analyzer root".to_string(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: true,
        selection_reason: None,
        children: vec![custom_root_node],
    };
    state.sessions.lock().unwrap().insert_test_analyzer_session(
        sweeper_desktop::session::AnalyzerSessionInput {
            id: "analyzer-custom-root-test".to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen,
            result: sweeper_core::analyzer::AnalyzerResult {
                root: tree_root,
                progress: sweeper_core::analyzer::AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours: 24,
        },
    );
    node_id
}

fn seed_custom_root_analyzer_ctx() -> CustomRootCtx {
    let state = make_state();
    let sandbox = Sandbox::new_in(sandbox_test_base());
    sandbox.assert_sentinel_present();
    let canary = CanaryTree::seed_external("gate-f-analyzer-custom-root").unwrap();
    let (root_path, inside, nested) = setup_custom_root_fs(&sandbox);
    canary.verify().unwrap();
    let node_id = insert_custom_root_session(&state, &root_path);
    CustomRootCtx {
        state,
        sandbox,
        canary,
        root_path,
        inside,
        nested,
        node_id,
    }
}

#[test]
fn analyzer_custom_path_root_requires_confirmation() {
    let ctx = seed_custom_root_analyzer_ctx();
    let err = commands_impl::preview_clean(
        &ctx.state,
        "analyzer-custom-root-test".to_string(),
        vec![ctx.node_id],
        None,
    )
    .unwrap_err();
    assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
}

#[test]
fn analyzer_custom_path_root_preserved_on_execute() {
    let ctx = seed_custom_root_analyzer_ctx();
    let preview = commands_impl::preview_clean(
        &ctx.state,
        "analyzer-custom-root-test".to_string(),
        vec![ctx.node_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    let logs_base = tempfile::tempdir().unwrap();
    let exec = commands_impl::execute_clean(
        &ctx.state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .unwrap();
    assert!(!exec.aborted, "abort_reason={:?}", exec.abort_reason);

    ctx.sandbox.assert_sentinel_present();
    assert!(
        ctx.root_path.exists(),
        "custom-path root directory must be preserved"
    );
    assert!(
        !ctx.inside.exists(),
        "root-level contents should be cleaned"
    );
    assert!(!ctx.nested.exists(), "nested contents should be cleaned");
    assert!(
        ctx.canary.verify().is_ok(),
        "external canary must remain intact"
    );
}

#[derive(Debug)]
struct FakeElevationProvider {
    elevated: bool,
    relaunch_outcome: std::sync::Mutex<Result<sweeper_core::elevation::RelaunchOutcome, String>>,
    relaunch_calls: std::sync::atomic::AtomicUsize,
    status_calls: std::sync::atomic::AtomicUsize,
}

impl FakeElevationProvider {
    fn new(
        elevated: bool,
        outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>,
    ) -> Self {
        Self {
            elevated,
            relaunch_outcome: std::sync::Mutex::new(outcome),
            relaunch_calls: std::sync::atomic::AtomicUsize::new(0),
            status_calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl sweeper_desktop::elevation::ElevationProvider for FakeElevationProvider {
    fn is_elevated(&self) -> Result<bool, String> {
        self.status_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.elevated)
    }

    fn relaunch(&self) -> Result<sweeper_core::elevation::RelaunchOutcome, String> {
        self.relaunch_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.relaunch_outcome.lock().unwrap().clone()
    }
}

#[allow(clippy::type_complexity)]
fn make_elevation_test_state(
    elevated: bool,
    relaunch_outcome: Result<sweeper_core::elevation::RelaunchOutcome, String>,
) -> (
    sweeper_desktop::AppState<FixedClock>,
    std::sync::Arc<FakeElevationProvider>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    std::sync::Arc<std::sync::Mutex<Option<i32>>>,
    std::sync::Arc<sweeper_desktop::elevation::SingleInstanceState>,
) {
    let provider = std::sync::Arc::new(FakeElevationProvider::new(elevated, relaunch_outcome));
    let exited = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let exit_code = std::sync::Arc::new(std::sync::Mutex::new(None));
    let exited_c = exited.clone();
    let exit_code_c = exit_code.clone();
    let exit_hook: std::sync::Arc<dyn Fn(i32) + Send + Sync> = std::sync::Arc::new(move |code| {
        exited_c.store(true, std::sync::atomic::Ordering::SeqCst);
        *exit_code_c.lock().unwrap() = Some(code);
    });
    static CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mutex_name = format!(r"Local\SweeperGateFElevation-{pid}-{nanos}-{n}");
    let guard = sweeper_core::elevation::SingleInstanceGuard::acquire_named(&mutex_name, 1_000)
        .expect("test should acquire single-instance mutex");
    let si = std::sync::Arc::new(sweeper_desktop::elevation::SingleInstanceState::with_guard(
        guard,
    ));
    let state = sweeper_desktop::AppState::with_clock(FixedClock::new(1_800_000_000))
        .with_elevation_test_hooks(provider.clone(), si.clone(), exit_hook);
    (state, provider, exited, exit_code, si)
}

#[test]
fn get_elevation_status_reports_provider_state() {
    let (state, provider, _exited, _code, _si) = make_elevation_test_state(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    );
    let got = commands_impl::get_elevation_status(&state).unwrap();
    assert!(!got.is_elevated);
    assert_eq!(
        provider
            .status_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    let (state2, _p2, _, _, _) =
        make_elevation_test_state(true, Ok(sweeper_core::elevation::RelaunchOutcome::Launched));
    let got2 = commands_impl::get_elevation_status(&state2).unwrap();
    assert!(got2.is_elevated);
}

#[test]
fn relaunch_elevated_when_already_elevated_returns_not_launched_without_side_effects() {
    let (state, provider, exited, exit_code, si) =
        make_elevation_test_state(true, Ok(sweeper_core::elevation::RelaunchOutcome::Launched));
    let res = commands_impl::relaunch_elevated(&state).unwrap();
    assert!(!res.launched);
    assert_eq!(
        provider
            .relaunch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert!(!exited.load(std::sync::atomic::Ordering::SeqCst));
    assert!(exit_code.lock().unwrap().is_none());
    // Guard should remain held/available for future operations.
    let _ = si; // state owns Arc
}

#[test]
fn relaunch_elevated_uac_cancel_returns_error_preserves_guard_and_does_not_exit() {
    let (state, provider, exited, exit_code, si) = make_elevation_test_state(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::CancelledByUser),
    );
    let err = commands_impl::relaunch_elevated(&state).unwrap_err();
    assert!(
        matches!(err, sweeper_contract::ContractError::InvalidInput { .. }),
        "expected sanitized user-cancel error, got {err:?}"
    );
    assert_eq!(
        provider
            .relaunch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(!exited.load(std::sync::atomic::Ordering::SeqCst));
    assert!(exit_code.lock().unwrap().is_none());
    // Guard must be preserved/re-acquired in state after cancel.
    assert!(
        si.is_held(),
        "single-instance guard should remain held after UAC cancel"
    );
}

#[test]
fn relaunch_elevated_success_releases_guard_invokes_launcher_returns_launched_and_triggers_exit() {
    let (state, provider, exited, exit_code, si) = make_elevation_test_state(
        false,
        Ok(sweeper_core::elevation::RelaunchOutcome::Launched),
    );
    let res = commands_impl::relaunch_elevated(&state).unwrap();
    assert!(res.launched);
    assert_eq!(
        provider
            .relaunch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert!(exited.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(*exit_code.lock().unwrap(), Some(0));
    // Guard handed off/released from this instance's state.
    assert!(
        !si.is_held(),
        "single-instance guard should be released from current instance on successful handoff"
    );
}

#[test]
fn start_scan_emits_started_progress_completed_with_expected_session_and_decimal_counters() {
    // Sandbox-only: disable all tiers and custom paths so no real user/system paths are touched.
    let state = state_with_temp_settings(|s| {
        s.enable_tier1 = false;
        s.enable_tier2 = false;
        s.enable_tier3 = false;
        s.custom_paths.clear();
    });
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::<
        sweeper_contract::ScanProgressEvent,
    >::new()));
    let events_cb = events.clone();

    let resp = commands_impl::start_scan(
        &state,
        None,
        Some(move |evt: sweeper_contract::ScanProgressEvent| {
            events_cb.lock().unwrap().push(evt);
        }),
    )
    .expect("start_scan should succeed");

    let events = events.lock().unwrap().clone();
    assert!(!events.is_empty(), "expected scan progress events");
    match &events[0] {
        sweeper_contract::ScanProgressEvent::Started { session_id } => {
            assert_eq!(session_id, &resp.session_id);
        }
        other => panic!("expected Started as first event, got {other:?}"),
    }
    match events.last().unwrap() {
        sweeper_contract::ScanProgressEvent::Completed { session_id } => {
            assert_eq!(session_id, &resp.session_id);
        }
        other => panic!("expected Completed as last event, got {other:?}"),
    }
}

#[test]
fn session_store_lock_not_held_during_inflight_scan() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let sandbox_scan = Sandbox::new();
    let scan_dir = sandbox_scan.create_dir("ScanRoot").unwrap();
    let _ = sandbox_scan
        .write_file("ScanRoot\\locktest.tmp", b"lock-scan-data")
        .unwrap();
    let state = Arc::new(state_with_temp_settings(|s| {
        s.enable_tier1 = false;
        s.enable_tier2 = false;
        s.enable_tier3 = false;
        s.custom_paths = vec![scan_dir.to_string_lossy().to_string()];
    }));
    let try_lock_ok = Arc::new(AtomicBool::new(false));
    let state_cb = Arc::clone(&state);
    let try_lock_ok_cb = Arc::clone(&try_lock_ok);
    let resp = commands_impl::start_scan(
        &state,
        None,
        Some(move |_evt: sweeper_contract::ScanProgressEvent| {
            if state_cb.sessions.try_lock().is_ok() {
                try_lock_ok_cb.store(true, Ordering::SeqCst);
            }
        }),
    )
    .expect("start_scan should succeed");
    assert!(!resp.session_id.is_empty());
    assert!(
        try_lock_ok.load(Ordering::SeqCst),
        "sessions lock must be acquirable during in-flight scan progress callbacks"
    );
}

#[test]
fn session_store_lock_not_held_during_inflight_execute_clean() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let state = Arc::new(make_state());
    let sandbox = Sandbox::new();
    let canary = CanaryTree::seed_external("gate-f-lock-clean").unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let file_path = sandbox
        .write_file("Temp\\deleteme-lock.tmp", b"hello-sweeper")
        .unwrap();
    seed_session(&state, &root, &file_path, 1_700_000_000);
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
    assert!(
        try_lock_ok.load(Ordering::SeqCst),
        "sessions lock must be acquirable during in-flight clean progress callbacks"
    );
}

#[test]
fn session_store_lock_not_held_during_inflight_analyzer() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let state = Arc::new(make_state());
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("A").unwrap();
    let _f1 = sandbox.write_file("A\\file1.txt", b"hello").unwrap();
    let _f2 = sandbox
        .write_file("A\\sub\\file2.bin", [0u8; 1024])
        .unwrap();

    let try_lock_ok = Arc::new(AtomicBool::new(false));
    let state_cb = Arc::clone(&state);
    let try_lock_ok_cb = Arc::clone(&try_lock_ok);
    let root_s = root.to_string_lossy().to_string();
    let resp = commands_impl::start_analyzer(
        &state,
        Some(root_s),
        Some(move |_evt: sweeper_contract::AnalyzerProgressEvent| {
            if state_cb.sessions.try_lock().is_ok() {
                try_lock_ok_cb.store(true, Ordering::SeqCst);
            }
        }),
    )
    .expect("start_analyzer should succeed");
    assert!(!resp.session_id.is_empty());
    assert!(
        try_lock_ok.load(Ordering::SeqCst),
        "sessions lock must be acquirable during in-flight analyzer progress callbacks"
    );
}
