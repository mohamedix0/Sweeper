mod common;

use sweeper_core::analyzer::AnalyzerNode;
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::AppState;
use sweeper_testkit::Sandbox;

fn start_r_state() -> (AppState<FixedClock>, Sandbox, String, String) {
    let state = common::make_state();
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
    (state, sandbox, resp.session_id, resp.root.id)
}

#[test]
fn analyzer_session_id_not_accepted_as_preview_token() {
    let (state, _sandbox, session_id, _root_id) = start_r_state();
    let err = commands_impl::execute_clean(&state, session_id, None, |_| {}).unwrap_err();
    common::assert_preview_token_invalid_err(err);
}

#[test]
fn analyzer_session_expired_rejected() {
    let (state, _sandbox, session_id, root_id) = start_r_state();
    state.clock.advance_secs(15 * 60 + 1);
    let err = common::preview_with_delete_confirmation(&state, &session_id, root_id).unwrap_err();
    common::assert_session_invalid_err(err);
}

#[test]
fn analyzer_session_invalidated_by_settings_change_rejected() {
    let state = common::make_state();
    let sandbox = Sandbox::new();
    let root = sandbox.create_dir("R2").unwrap();
    let _ = sandbox.write_file("R2\\ok.txt", b"ok").unwrap();
    let resp = commands_impl::start_analyzer::<_, fn(sweeper_contract::AnalyzerProgressEvent)>(
        &state,
        Some(root.to_string_lossy().to_string()),
        None,
    )
    .unwrap();
    let mut settings = state.settings.lock().unwrap();
    let mut s = settings.get();
    s.min_age_hours = 10;
    let _ = settings.update(s);
    drop(settings);
    let err = common::preview_with_delete_confirmation(&state, &resp.session_id, resp.root.id)
        .unwrap_err();
    common::assert_session_invalid_err(err);
}

fn seed_unknown_dir_session(state: &AppState<FixedClock>) -> String {
    let unknown_dir = AnalyzerNode {
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
    let root = AnalyzerNode {
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
    common::insert_analyzer_session(state, "analyzer-unknown-test", root, 24);
    "node-unknown-dir".to_string()
}

#[test]
fn analyzer_unknown_node_not_selectable_and_rejected_when_unknown_deletions_disabled() {
    let state = common::make_state();
    assert!(!state.settings.lock().unwrap().get().allow_delete_unknown);
    let node_id = seed_unknown_dir_session(&state);
    let err = commands_impl::preview_clean(
        &state,
        "analyzer-unknown-test".to_string(),
        vec![node_id],
        None,
    )
    .unwrap_err();
    common::assert_invalid_input_err(err);
}

fn seed_unknown_selectable_analyzer_session(state: &AppState<FixedClock>) -> String {
    let sandbox = Sandbox::new_in(common::sandbox_test_base());
    let uroot = sandbox.create_dir("UnknownRoot").unwrap();
    let mut unknown_node = common::base_analyzer_node(common::AnalyzerNodeSeed {
        id: "node-unknown-custom",
        path: &uroot.to_string_lossy(),
        name: "UnknownRoot",
        is_dir: true,
    });
    unknown_node.risk = sweeper_core::Risk::Unknown;
    unknown_node.description = "Unknown".to_string();
    unknown_node.selectable = true;
    let node_id = unknown_node.id.clone();
    let mut tree_root = common::base_analyzer_node(common::AnalyzerNodeSeed {
        id: "node-tree-root-u",
        path: &uroot.to_string_lossy(),
        name: "Root",
        is_dir: true,
    });
    tree_root.item_count = 1;
    tree_root.children = vec![unknown_node];
    common::insert_analyzer_session(state, "analyzer-unknown-ok", tree_root, 24);
    std::mem::forget(sandbox);
    node_id
}

#[test]
fn typed_confirmation_rejected_for_unknown_analyzer_items() {
    let state = common::state_with_temp_settings(|s| s.allow_delete_unknown = true);
    let node_id = seed_unknown_selectable_analyzer_session(&state);
    let err = commands_impl::preview_clean(
        &state,
        "analyzer-unknown-ok".to_string(),
        vec![node_id],
        Some(sweeper_contract::PreviewConfirmationsDto {
            typed_confirmation: Some("wrong".to_string()),
        }),
    )
    .unwrap_err();
    common::assert_invalid_input_err(err);
}

#[test]
fn typed_confirmation_accepted_for_unknown_analyzer_items() {
    let state = common::state_with_temp_settings(|s| s.allow_delete_unknown = true);
    let node_id = seed_unknown_selectable_analyzer_session(&state);
    let token = common::preview_accepts_delete_confirmation(
        &state,
        common::SessionItem {
            session_id: "analyzer-unknown-ok",
            item_id: &node_id,
        },
    );
    assert!(!token.is_empty());
}
