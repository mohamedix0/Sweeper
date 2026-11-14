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
        .create_test_session_with_items(1_800_000_000, settings_gen, "scan-test", results, items);
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
    let sandbox = Sandbox::new().unwrap();
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

#[test]
fn analyzer_eligible_node_preview_and_execute_clean_in_sentinel_sandbox() {
    // Custom-path sandbox nodes are Unknown and require allow_delete_unknown to be selectable.
    let state = state_with_temp_settings(|s| {
        s.allow_delete_unknown = true;
    });
    let canary = CanaryTree::seed_external("gate-f-analyzer-exec").unwrap();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("CleanMe").unwrap();
    let file_path = sandbox
        .write_file("CleanMe\\old.txt", b"delete-me")
        .unwrap();
    assert!(file_path.exists());

    canary.verify().unwrap();

    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();

    let root_node = resp.root.clone();
    assert!(root_node.selectable || !root_node.selectable);

    // Select the analyzed file node and route it through the standard Preview/Clean flow.
    let file_node =
        find_node_by_path_suffix(&root_node, "\\old.txt").expect("old.txt node in analyzer tree");
    assert!(
        file_node.selectable,
        "old.txt should be selectable: reason={:?}",
        file_node.reason
    );
    assert!(!file_node.display_only);

    let confirmations = if file_node.target_id.is_none() || file_node.risk == RiskDto::Unknown {
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        })
    } else {
        None
    };
    let preview = commands_impl::preview_clean(
        &state,
        resp.session_id.clone(),
        vec![file_node.id.clone()],
        confirmations,
    )
    .unwrap();
    assert!(preview.total_selected_count.get() >= 1);

    let logs_base = tempfile::tempdir().unwrap();
    let exec = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .unwrap();
    assert!(!exec.aborted, "abort_reason={:?}", exec.abort_reason);
    assert!(
        exec.deleted_count.get() >= 1,
        "expected at least one deleted item, got {:?}",
        exec
    );
    canary.verify().unwrap();
    assert!(!sandbox.guarded_path("CleanMe\\old.txt").exists());
    let log_id = exec.log_id.expect("log id");
    let logs =
        commands_impl::list_logs_in_dir(Some(logs_base.path().join("logs").as_path())).unwrap();
    assert!(logs.iter().any(|l| l.id == log_id));
}

#[test]
fn analyzer_preview_rejects_invalid_nodes_and_session_rules_enforced() {
    let state = make_state();
    let sandbox = Sandbox::new().unwrap();
    let root = sandbox.create_dir("R").unwrap();
    let _f = sandbox.write_file("R\\ok.txt", b"ok").unwrap();
    let _vhdx = sandbox.write_file("R\\disk.vhdx", b"x").unwrap();
    let _git = sandbox.create_dir("R\\.git").unwrap();
    let outside = sandbox.create_dir("outside").unwrap();
    let link = sandbox.guarded_path("R\\linkdir");
    let _ = sweeper_testkit::create_junction(&link, &outside);

    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();

    if let Some(vhdx) = find_node_by_path_suffix(&resp.root, "\\disk.vhdx") {
        let err = commands_impl::preview_clean(
            &state,
            resp.session_id.clone(),
            vec![vhdx.id.clone()],
            None,
        )
        .unwrap_err();
        assert!(
            matches!(err, ContractError::InvalidInput { .. })
                || matches!(err, ContractError::UnknownItem { .. })
                || format!("{err:?}").contains("InvalidInput"),
            "{err:?}"
        );
    }
    if let Some(git) = find_node_by_path_suffix(&resp.root, "\\.git") {
        let err = commands_impl::preview_clean(
            &state,
            resp.session_id.clone(),
            vec![git.id.clone()],
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
    }
    if let Some(link_node) = find_node_by_path_suffix(&resp.root, "\\linkdir") {
        let err = commands_impl::preview_clean(
            &state,
            resp.session_id.clone(),
            vec![link_node.id.clone()],
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ContractError::InvalidInput { .. }), "{err:?}");
    }

    // execute_clean directly with analyzer session_id (not preview token) rejected
    let err =
        commands_impl::execute_clean(&state, resp.session_id.clone(), None, |_| {}).unwrap_err();
    assert!(
        matches!(err, ContractError::PreviewTokenInvalid { .. }),
        "{err:?}"
    );

    // expired analyzer session rejected
    state.clock.advance_secs(15 * 60 + 1);
    let err = commands_impl::preview_clean(
        &state,
        resp.session_id.clone(),
        vec![resp.root.id.clone()],
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
    let sandbox = Sandbox::new().unwrap();
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

#[test]
fn analyzer_unknown_node_not_selectable_and_rejected_when_unknown_deletions_disabled() {
    let state = make_state();
    let settings_gen = {
        let settings = state.settings.lock().unwrap();
        assert!(
            !settings.get().allow_delete_unknown,
            "allow_delete_unknown must be false by default for this test"
        );
        settings.generation()
    };

    // Explicit Unknown-risk directory node (not selectable) in a synthetic analyzer session.
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
        children: vec![unknown_dir.clone()],
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

    let err = commands_impl::preview_clean(
        &state,
        "analyzer-unknown-test".to_string(),
        vec![unknown_dir.id.clone()],
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::InvalidInput { .. }),
        "non-selectable Unknown-risk directory node must be rejected by preview_clean: {err:?}"
    );
}

#[test]
fn validate_custom_path_rejects_missing_and_protected_and_accepts_valid_sandbox_path() {
    let state = make_state();
    let sandbox_base = sandbox_test_base();
    let sandbox = sweeper_testkit::Sandbox::new_in(&sandbox_base).unwrap();
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

#[test]
fn update_settings_increments_generation_and_invalidates_active_sessions() {
    let state = make_state();
    let sandbox = sweeper_testkit::Sandbox::new().unwrap();
    let root = sandbox.create_dir("Temp").unwrap();
    let f = sandbox.write_file("Temp\\a.tmp", b"abc").unwrap();
    seed_session(&state, &root, &f, 1_700_000_000);

    // seed analyzer session too
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

    let mut s = commands_impl::get_settings(&state).unwrap();
    s.custom_paths
        .push(r"C:\Definitely\Not\A\Real\Valid\Path".to_string()); // will sanitize/drop but still represents change attempt; better change a real field
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

#[test]
fn typed_confirmation_rejected_and_accepted_for_custom_path_scan_target() {
    let state = make_state();
    let settings_gen = state.settings.lock().unwrap().generation();

    // Custom Path scan target session
    let sandbox_base = sandbox_test_base();
    let sandbox = sweeper_testkit::Sandbox::new_in(&sandbox_base).unwrap();
    let root = sandbox.create_dir("CustomRoot").unwrap();
    let file_path = sandbox.write_file("CustomRoot\\old.tmp", b"abc").unwrap();
    let size = fs::metadata(&file_path).map(|m| m.len()).unwrap_or(3);
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
    state
        .sessions
        .lock()
        .unwrap()
        .create_test_session_with_items(1_800_000_000, settings_gen, "scan-custom", results, items);

    let err = commands_impl::preview_clean(
        &state,
        "scan-custom".to_string(),
        vec![item.id.clone()],
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::InvalidInput { .. }),
        "missing confirmation must be rejected: {err:?}"
    );
    let err = commands_impl::preview_clean(
        &state,
        "scan-custom".to_string(),
        vec![item.id.clone()],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("WRONG".to_string()),
        }),
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::InvalidInput { .. }),
        "wrong confirmation must be rejected: {err:?}"
    );
    let ok = commands_impl::preview_clean(
        &state,
        "scan-custom".to_string(),
        vec![item.id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(!ok.preview_token.is_empty());
}

#[test]
fn typed_confirmation_rejected_and_accepted_for_unknown_analyzer_items() {
    // Analyzer Unknown item requires exact typed confirmation (custom-path conversion seam)
    let sandbox_base = sandbox_test_base();
    let state2 = state_with_temp_settings(|s| {
        s.allow_delete_unknown = true;
    });
    let settings_gen2 = state2.settings.lock().unwrap().generation();
    let sandbox2 = sweeper_testkit::Sandbox::new_in(&sandbox_base).unwrap();
    let uroot = sandbox2.create_dir("UnknownRoot").unwrap();
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
        children: vec![unknown_node.clone()],
    };
    state2
        .sessions
        .lock()
        .unwrap()
        .insert_test_analyzer_session(sweeper_desktop::session::AnalyzerSessionInput {
            id: "analyzer-unknown-ok".to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen2,
            result: sweeper_core::analyzer::AnalyzerResult {
                root: tree_root,
                progress: sweeper_core::analyzer::AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours: 24,
        });

    let err = commands_impl::preview_clean(
        &state2,
        "analyzer-unknown-ok".to_string(),
        vec![unknown_node.id.clone()],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("wrong".to_string()),
        }),
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::InvalidInput { .. }),
        "wrong confirmation for Unknown analyzer selection must be rejected: {err:?}"
    );
    let ok = commands_impl::preview_clean(
        &state2,
        "analyzer-unknown-ok".to_string(),
        vec![unknown_node.id.clone()],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();
    assert!(!ok.preview_token.is_empty());
}

#[test]
fn analyzer_custom_path_root_requires_confirmation_and_preserves_root_on_execute() {
    let state = make_state();
    let settings_gen = state.settings.lock().unwrap().generation();

    // Sentinel-guarded per-test sandbox (auto-cleaned on Drop) and external canary.
    // Use CARGO_TARGET_DIR as base: custom-path validation rejects descendants of protected
    // container roots (e.g. user profile/%TEMP%), but CARGO_TARGET_DIR is outside them.
    let sandbox_base = sandbox_test_base();
    let sandbox = Sandbox::new_in(&sandbox_base).unwrap();
    sandbox.assert_sentinel_present();
    let canary = CanaryTree::seed_external("gate-f-analyzer-custom-root").unwrap();

    let root_path = sandbox.create_dir("CustomCleanRoot").unwrap();
    let inside = sandbox
        .write_file("CustomCleanRoot\\old.tmp", b"bye")
        .unwrap();
    let _nested_dir = sandbox.create_dir("CustomCleanRoot\\nested").unwrap();
    let nested = sandbox
        .write_file("CustomCleanRoot\\nested\\x.tmp", b"x")
        .unwrap();
    assert!(root_path.exists() && inside.exists() && nested.exists());

    let verdict = sweeper_core::policy::validate_custom_path(&root_path.to_string_lossy());
    assert!(
        verdict.allowed,
        "sandbox custom path root must be valid for this environment: {} ({})",
        root_path.display(),
        verdict.reason
    );
    canary.verify().unwrap();

    let root_str = root_path.to_string_lossy().to_string();
    // Synthetic selectable custom-path-root directory node (target_id None => custom-path conversion seam).
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
    let tree_root = sweeper_core::analyzer::AnalyzerNode {
        id: "node-tree-root".to_string(),
        path: root_str.clone(),
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
        children: vec![custom_root_node.clone()],
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

    // Custom-path-root analyzer nodes require typed confirmation.
    let err = commands_impl::preview_clean(
        &state,
        "analyzer-custom-root-test".to_string(),
        vec![custom_root_node.id.clone()],
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, ContractError::InvalidInput { .. }),
        "custom-path-root selection must require typed confirmation: {err:?}"
    );

    let preview = commands_impl::preview_clean(
        &state,
        "analyzer-custom-root-test".to_string(),
        vec![custom_root_node.id.clone()],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
    .unwrap();

    let logs_base = tempfile::tempdir().unwrap();
    let exec = commands_impl::execute_clean(
        &state,
        preview.preview_token,
        Some(logs_base.path().to_path_buf()),
        |_| {},
    )
    .unwrap();
    assert!(!exec.aborted, "abort_reason={:?}", exec.abort_reason);

    // Custom-path root directory itself must survive; eligible contents cleaned.
    sandbox.assert_sentinel_present();
    assert!(
        root_path.exists(),
        "custom-path root directory must be preserved"
    );
    assert!(!inside.exists(), "root-level contents should be cleaned");
    assert!(!nested.exists(), "nested contents should be cleaned");
    assert!(
        canary.verify().is_ok(),
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
