#![allow(dead_code)]

use std::path::Path;

use sweeper_contract::{
    AnalyzerNodeDto, ExecuteCleanResponse, PreviewCleanResponse, PreviewConfirmationsDto, RiskDto,
};
use sweeper_core::analyzer::{AnalyzerNode, AnalyzerProgress, AnalyzerResult};
use sweeper_core::Risk;
use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::commands_impl;
use sweeper_desktop::session::AnalyzerSessionInput;
use sweeper_desktop::AppState;

pub fn find_node_by_path_suffix<'a>(
    node: &'a AnalyzerNodeDto,
    suffix: &str,
) -> Option<&'a AnalyzerNodeDto> {
    if node
        .path
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|c| find_node_by_path_suffix(c, suffix))
}

pub fn insert_analyzer_session(
    state: &AppState<FixedClock>,
    session_id: &str,
    root: AnalyzerNode,
    min_age_hours: u32,
) {
    let settings_gen = state.settings.lock().unwrap().generation();
    state
        .sessions
        .lock()
        .unwrap()
        .insert_test_analyzer_session(AnalyzerSessionInput {
            id: session_id.to_string(),
            created_at: 1_800_000_000,
            settings_generation: settings_gen,
            result: AnalyzerResult {
                root,
                progress: AnalyzerProgress::default(),
                cancelled: false,
            },
            min_age_hours,
        });
}

pub fn assert_unknown_file_not_selectable(root_node: &AnalyzerNodeDto) {
    let file_node =
        find_node_by_path_suffix(root_node, "\\StaleCache\\old.txt").expect("old.txt node");
    assert!(!file_node.selectable, "reason={:?}", file_node.reason);
    assert_eq!(
        file_node.reason.as_deref(),
        Some("Unknown files are not selectable; only eligible directories can be selected"),
        "{:?}",
        file_node.reason
    );
}

pub fn assert_stale_cache_dir_selectable(root_node: &AnalyzerNodeDto) -> AnalyzerNodeDto {
    let node = find_node_by_path_suffix(root_node, "\\StaleCache").expect("StaleCache node");
    assert!(node.is_dir, "StaleCache must be a directory node");
    assert!(node.selectable, "reason={:?}", node.reason);
    assert!(!node.display_only, "StaleCache must not be display_only");
    let treated_as_custom = node.target_id.is_none() || node.risk == RiskDto::Unknown;
    assert!(treated_as_custom, "{node:?}");
    node.clone()
}

pub fn exec_analyzer_delete_confirmation(
    state: &AppState<FixedClock>,
    session_id: &str,
    node_id: String,
) -> ExecuteCleanResponse {
    let preview = preview_with_delete_confirmation(state, session_id, node_id).unwrap();
    assert!(preview.total_selected_count.0 >= 1);
    let logs_base = tempfile::tempdir().unwrap();
    execute_preview_token(state, preview.preview_token, logs_base.path())
}

pub struct AnalyzerNodeSeed<'a> {
    pub id: &'a str,
    pub path: &'a str,
    pub name: &'a str,
    pub is_dir: bool,
}

pub fn base_analyzer_node(seed: AnalyzerNodeSeed<'_>) -> AnalyzerNode {
    AnalyzerNode {
        id: seed.id.to_string(),
        path: seed.path.to_string(),
        name: seed.name.to_string(),
        is_dir: seed.is_dir,
        size_bytes: 0,
        item_count: 0,
        risk: Risk::Safe,
        description: String::new(),
        tier: None,
        target_id: None,
        is_link_point: false,
        is_cloud_placeholder: false,
        display_only: false,
        selectable: false,
        selection_reason: None,
        children: Vec::new(),
    }
}

pub fn preview_with_delete_confirmation(
    state: &AppState<FixedClock>,
    session_id: &str,
    item_id: String,
) -> Result<PreviewCleanResponse, sweeper_contract::ContractError> {
    commands_impl::preview_clean(
        state,
        session_id.to_string(),
        vec![item_id],
        Some(PreviewConfirmationsDto {
            typed_confirmation: Some("DELETE".to_string()),
        }),
    )
}

pub fn execute_preview_token(
    state: &AppState<FixedClock>,
    preview_token: String,
    logs_base: &Path,
) -> ExecuteCleanResponse {
    commands_impl::execute_clean(state, preview_token, Some(logs_base.to_path_buf()), |_| {})
        .unwrap()
}
