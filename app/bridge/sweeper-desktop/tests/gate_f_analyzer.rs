mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sweeper_contract::{ContractError, RiskDto};
use sweeper_desktop::commands_impl;
use sweeper_testkit::{CanaryTree, Sandbox};

fn assert_invalid(err: ContractError) {
    common::assert_invalid_input_err(err);
}

#[test]
fn start_analyzer_on_sandbox_returns_tree_marks_non_selectable_and_populates_session() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("A").unwrap();
    let _ = sandbox.write_file("A\\file1.txt", b"hello").unwrap();
    let _ = sandbox.write_file("A\\sub\\file2.bin", [0u8; 10]).unwrap();
    let _ = sandbox.write_file("A\\disk.vhdx", b"vhdx-bytes").unwrap();
    let _ = sandbox.create_dir("A\\.git").unwrap();
    let _ = sandbox.write_file("A\\.git\\config", b"[core]\n").unwrap();
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
    assert!(resp.total_bytes.0 >= 15);
    assert!(resp.total_items.0 >= 2);
    let vhdx = common::find_node_by_path_suffix(&resp.root, "\\disk.vhdx").expect("vhdx");
    assert!(vhdx.display_only, "vhdx must be display_only");
    assert!(!vhdx.selectable, "vhdx must not be selectable");
    let git = common::find_node_by_path_suffix(&resp.root, "\\.git").expect(".git");
    assert_eq!(git.risk, RiskDto::Protected);
    assert!(!git.selectable, ".git must not be selectable");
    if let Some(link_node) = common::find_node_by_path_suffix(&resp.root, "\\linkdir") {
        assert!(link_node.is_link_point);
        assert!(!link_node.selectable);
    }
    let explained = commands_impl::explain_item(&state, resp.root.id.clone()).unwrap();
    assert!(!explained.description.is_empty());
}

#[test]
fn analyzer_eligible_node_preview_and_execute_clean_in_sentinel_sandbox() {
    let state = common::state_with_temp_settings(|s| s.allow_delete_unknown = true);
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
    common::assert_unknown_file_not_selectable(&resp.root);
    let stale_cache_node = common::assert_stale_cache_dir_selectable(&resp.root);
    let exec =
        common::exec_analyzer_delete_confirmation(&state, &resp.session_id, stale_cache_node.id);
    assert!(!exec.aborted, "abort_reason={:?}", exec.abort_reason);
    assert!(exec.deleted_count.0 >= 1, "expected deleted: {exec:?}");
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

fn start_reject_ctx(
    state: &sweeper_desktop::AppState<sweeper_desktop::clock::FixedClock>,
    root: &std::path::Path,
) -> sweeper_contract::StartAnalyzerResponse {
    commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap()
}

#[test]
fn analyzer_preview_rejects_invalid_nodes() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("R").unwrap();
    let _ = sandbox.write_file("R\\ok.txt", b"ok").unwrap();
    let _ = sandbox.write_file("R\\disk.vhdx", b"x").unwrap();
    let _ = sandbox.create_dir("R\\.git").unwrap();
    let outside = sandbox.create_dir("outside").unwrap();
    let link = sandbox.guarded_path("R\\linkdir");
    let _ = sweeper_testkit::create_junction(&link, &outside);
    let resp = start_reject_ctx(&state, &root);
    if let Some(v) = common::find_node_by_path_suffix(&resp.root, "\\disk.vhdx") {
        let err =
            commands_impl::preview_clean(&state, resp.session_id.clone(), vec![v.id.clone()], None)
                .unwrap_err();
        common::assert_unknown_or_invalid(err);
    }
    if let Some(g) = common::find_node_by_path_suffix(&resp.root, "\\.git") {
        let err =
            commands_impl::preview_clean(&state, resp.session_id.clone(), vec![g.id.clone()], None)
                .unwrap_err();
        assert_invalid(err);
    }
    if let Some(l) = common::find_node_by_path_suffix(&resp.root, "\\linkdir") {
        let err = commands_impl::preview_clean(&state, resp.session_id, vec![l.id.clone()], None)
            .unwrap_err();
        assert_invalid(err);
    }
}

fn seed_custom_root_session(
    state: &sweeper_desktop::AppState<sweeper_desktop::clock::FixedClock>,
    root_path: &std::path::Path,
) {
    let mut child = common::base_analyzer_node(common::AnalyzerNodeSeed {
        id: "node-custom-root",
        path: &root_path.to_string_lossy(),
        name: "CustomCleanRoot",
        is_dir: true,
    });
    child.selectable = true;
    child.size_bytes = 4;
    child.item_count = 2;
    let mut tree_root = common::base_analyzer_node(common::AnalyzerNodeSeed {
        id: "node-tree-root",
        path: &root_path.to_string_lossy(),
        name: "CustomCleanRoot",
        is_dir: true,
    });
    tree_root.selectable = true;
    tree_root.size_bytes = 4;
    tree_root.item_count = 3;
    tree_root.children = vec![child];
    common::insert_analyzer_session(state, "analyzer-custom-root-test", tree_root, 24);
}

struct CustomRootSetup {
    _sandbox: Sandbox,
    canary: CanaryTree,
    root_path: std::path::PathBuf,
    inside: std::path::PathBuf,
    nested: std::path::PathBuf,
}

fn setup_custom_root() -> CustomRootSetup {
    let sandbox = Sandbox::new_in(common::sandbox_test_base());
    sandbox.assert_sentinel_present();
    let canary = CanaryTree::seed_external("gate-f-analyzer-custom-root").unwrap();
    let root_path = sandbox.create_dir("CustomCleanRoot").unwrap();
    let inside = sandbox
        .write_file("CustomCleanRoot\\old.tmp", b"bye")
        .unwrap();
    let _ = sandbox.create_dir("CustomCleanRoot\\nested").unwrap();
    let nested = sandbox
        .write_file("CustomCleanRoot\\nested\\x.tmp", b"x")
        .unwrap();
    let verdict = sweeper_core::policy::validate_custom_path(&root_path.to_string_lossy());
    assert!(
        verdict.allowed,
        "{} ({})",
        root_path.display(),
        verdict.reason
    );
    canary.verify().unwrap();
    CustomRootSetup {
        _sandbox: sandbox,
        canary,
        root_path,
        inside,
        nested,
    }
}

#[test]
fn analyzer_custom_path_root_requires_confirmation() {
    let state = common::make_state();
    let setup = setup_custom_root();
    seed_custom_root_session(&state, &setup.root_path);
    let err = commands_impl::preview_clean(
        &state,
        "analyzer-custom-root-test".to_string(),
        vec!["node-custom-root".to_string()],
        None,
    )
    .unwrap_err();
    assert_invalid(err);
}

#[test]
fn analyzer_custom_path_root_preserved_on_execute() {
    let state = common::make_state();
    let setup = setup_custom_root();
    seed_custom_root_session(&state, &setup.root_path);
    let preview = common::preview_with_delete_confirmation(
        &state,
        "analyzer-custom-root-test",
        "node-custom-root".to_string(),
    )
    .unwrap();
    let logs_base = tempfile::tempdir().unwrap();
    let exec = common::execute_preview_token(&state, preview.preview_token, logs_base.path());
    assert!(!exec.aborted, "abort_reason={:?}", exec.abort_reason);
    assert!(
        setup.root_path.exists(),
        "custom-path root must be preserved"
    );
    assert!(
        !setup.inside.exists(),
        "root-level contents should be cleaned"
    );
    assert!(!setup.nested.exists(), "nested contents should be cleaned");
    setup.canary.verify().unwrap();
}

#[test]
fn session_store_lock_not_held_during_inflight_analyzer() {
    let state = Arc::new(common::make_state());
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("A").unwrap();
    let _ = sandbox.write_file("A\\file1.txt", b"hello").unwrap();
    let _ = sandbox
        .write_file("A\\sub\\file2.bin", [0u8; 1024])
        .unwrap();
    let try_lock_ok = Arc::new(AtomicBool::new(false));
    let state_cb = Arc::clone(&state);
    let try_lock_ok_cb = Arc::clone(&try_lock_ok);
    let resp = commands_impl::start_analyzer(
        &state,
        Some(root.to_string_lossy().to_string()),
        Some(move |_evt: sweeper_contract::AnalyzerProgressEvent| {
            if state_cb.sessions.try_lock().is_ok() {
                try_lock_ok_cb.store(true, Ordering::SeqCst);
            }
        }),
    )
    .expect("start_analyzer should succeed");
    assert!(!resp.session_id.is_empty());
    assert!(try_lock_ok.load(Ordering::SeqCst));
}
