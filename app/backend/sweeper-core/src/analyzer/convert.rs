use super::*;
use crate::paths::CanonicalPath;
use crate::policy;
use crate::remover::{capture_file_identity, ValidatedItem};
use crate::scanner;
use std::collections::{HashMap, HashSet};
use sweeper_knowledge::{embedded_targets, Risk, Scope, TargetDefinition};

/// A single analyzer node selection converted for Preview/Clean routing.
#[derive(Debug, Clone)]
pub struct AnalyzerPreviewItem {
    /// Original analyzer node id selected by the client.
    pub selected_node_id: String,
    /// Validated item for `remover::execute_clean`.
    pub validated_item: ValidatedItem,
    /// Group id for preview breakdown (target id or `custom-path`).
    pub group_id: String,
    /// Human-readable group name for preview breakdown.
    pub group_name: String,
    /// Estimated/selected size in bytes.
    pub size_bytes: u64,
    /// Whether typed confirmation is required for this selection.
    pub requires_typed_confirmation: bool,
}

/// Convert selected eligible Analyzer node IDs into validated items for Preview/Clean routing.
///
/// Performs explicit safety re-validation and resolves allowlist target roots internally so bridge
/// layers do not depend on `sweeper-winfs` target resolution.
pub fn build_analyzer_preview_items(
    result: &AnalyzerResult,
    selected_node_ids: &[String],
) -> Result<Vec<AnalyzerPreviewItem>, String> {
    let index = index_nodes(&result.root);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for id in selected_node_ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = index
            .get(id)
            .ok_or_else(|| format!("selected analyzer node not found: {id}"))?;

        validate_node_convertible(node)?;

        let item = if let Some(tid) = &node.target_id {
            build_target_preview_item(node, tid)?
        } else {
            build_custom_path_preview_item(node)?
        };
        out.push(item);
    }

    Ok(out)
}

/// Deepest allowlist target root that contains the selected node.
fn find_matching_root<'a>(
    target_roots: &'a [CanonicalPath],
    node_canon: &CanonicalPath,
) -> Option<&'a CanonicalPath> {
    target_roots
        .iter()
        .filter(|r| r.is_self_or_ancestor_of(node_canon))
        .max_by_key(|r| r.segment_count())
}

/// Human-readable group name: description when present, otherwise the target name.
fn target_group_name(target: &TargetDefinition) -> String {
    if target.description.is_empty() {
        target.name.clone()
    } else {
        target.description.clone()
    }
}

/// Build the Preview item for an allowlist target selection.
fn build_target_preview_item(
    node: &AnalyzerNode,
    tid: &String,
) -> Result<AnalyzerPreviewItem, String> {
    let target = embedded_targets()
        .into_iter()
        .find(|t| &t.id == tid)
        .ok_or_else(|| format!("allowlist target missing for node {}: {tid}", node.id))?;
    let target_roots = annotate::resolve_target_roots_for_def(&target);
    let node_canon = CanonicalPath::parse_lexical(&node.path)
        .map_err(|e| format!("selected node path invalid for conversion: {e}"))?;
    let matching_root = find_matching_root(&target_roots, &node_canon);
    let request = TargetValidatedItemRequest {
        node,
        tid,
        target: &target,
        matching_root,
        node_canon: &node_canon,
    };
    let validated_item = build_target_validated_item(&request)?
        .with_identity_binding(capture_file_identity(std::path::Path::new(&node.path)));

    Ok(AnalyzerPreviewItem {
        selected_node_id: node.id.clone(),
        validated_item,
        group_id: tid.clone(),
        group_name: target_group_name(&target),
        size_bytes: node.size_bytes,
        requires_typed_confirmation: false,
    })
}

/// Inputs needed to route an allowlist target node to its validated-item constructor.
struct TargetValidatedItemRequest<'a> {
    node: &'a AnalyzerNode,
    tid: &'a String,
    target: &'a TargetDefinition,
    matching_root: Option<&'a CanonicalPath>,
    node_canon: &'a CanonicalPath,
}

/// Decide between root-style and ordinary leaf construction for a target node.
fn build_target_validated_item(
    request: &TargetValidatedItemRequest<'_>,
) -> Result<ValidatedItem, String> {
    if request
        .matching_root
        .is_some_and(|r| r.is_same_path(request.node_canon))
    {
        // Annotate marks a resolved target root itself as selectable.
        // Route that selection through a root-style constructor instead of
        // failing the inside-root check in `from_allowlisted_scan`.
        build_cache_root_item(request.node, request.tid, request.target.scope)
    } else {
        build_allowlisted_leaf_item(request.node, request.tid, request.matching_root)
    }
}

/// Build a selectable target root: whole-cache unit or contents-only allowlisted root.
fn build_cache_root_item(
    node: &AnalyzerNode,
    tid: &str,
    scope: Scope,
) -> Result<ValidatedItem, String> {
    if !node.is_dir {
        return Err(format!(
            "selected target root is not a directory: {}",
            node.path
        ));
    }
    let built = if scope == Scope::WholeCache {
        ValidatedItem::from_whole_cache_root(
            node.id.clone(),
            tid.to_owned(),
            node.path.clone(),
            node.size_bytes,
        )
    } else {
        ValidatedItem::from_allowlisted_root(
            node.id.clone(),
            tid.to_owned(),
            node.path.clone(),
            node.size_bytes,
        )
    };
    built.map_err(|e| format!("selected target root rejected by allowlist conversion: {e}"))
}

/// Build the ordinary `ValidatedItem` for a node strictly inside an allowlist root.
fn build_allowlisted_leaf_item(
    node: &AnalyzerNode,
    tid: &String,
    matching_root: Option<&CanonicalPath>,
) -> Result<ValidatedItem, String> {
    let target_root = matching_root.map(|r| r.to_string()).ok_or_else(|| {
        format!(
            "failed to resolve matching allowlist target root for {tid} and node {}",
            node.path
        )
    })?;
    ValidatedItem::from_allowlisted_scan(
        node.id.clone(),
        tid.clone(),
        target_root,
        node.path.clone(),
        node.size_bytes,
        None,
    )
    .map_err(|e| format!("selected node rejected by allowlist conversion: {e}"))
}

/// Build the Preview item for a non-target custom-path selection (typed confirmation required).
fn build_custom_path_preview_item(node: &AnalyzerNode) -> Result<AnalyzerPreviewItem, String> {
    let validated_item = build_custom_path_validated_item(node)?
        .with_identity_binding(capture_file_identity(std::path::Path::new(&node.path)));

    Ok(AnalyzerPreviewItem {
        selected_node_id: node.id.clone(),
        validated_item,
        group_id: "custom-path".to_string(),
        group_name: "Custom Path".to_string(),
        size_bytes: node.size_bytes,
        requires_typed_confirmation: true,
    })
}

/// Validate a non-target node as a custom-path root and construct its `ValidatedItem`.
fn build_custom_path_validated_item(node: &AnalyzerNode) -> Result<ValidatedItem, String> {
    if !node.is_dir {
        return Err(format!(
            "selected non-target node must be a directory custom-path root candidate: {}",
            node.path
        ));
    }
    let verdict = policy::validate_custom_path(&node.path);
    if !verdict.allowed {
        return Err(format!(
            "selected custom-path candidate rejected: {} ({})",
            node.path, verdict.reason
        ));
    }
    if custom_path_rejected_by_safety_policy(node) {
        return Err(format!(
            "selected custom-path candidate rejected by safety policy: {}",
            node.path
        ));
    }
    ValidatedItem::from_custom_path_root(node.id.clone(), node.path.clone(), node.size_bytes)
        .map_err(|e| format!("selected custom-path candidate rejected: {e}"))
}

/// Safety-policy rejection predicate for custom-path root candidates.
fn custom_path_rejected_by_safety_policy(node: &AnalyzerNode) -> bool {
    matches!(node.risk, Risk::Protected)
        || annotate::is_vhdx_path(&node.path)
        || node.is_link_point
        || node.is_cloud_placeholder
}

/// Convert selected eligible Analyzer node IDs into validated scan items for Preview/Clean routing.
///
/// Compatibility helper retained for callers that only need `scanner::ScanItem`s.
pub fn build_scan_items_from_analyzer(
    result: &AnalyzerResult,
    selected_node_ids: &[String],
) -> Result<Vec<scanner::ScanItem>, String> {
    let preview_items = build_analyzer_preview_items(result, selected_node_ids)?;
    let mut out = Vec::with_capacity(preview_items.len());
    let mut seq: u64 = 0;
    for item in preview_items {
        seq += 1;
        let is_custom = item.group_id == "custom-path";
        let target_id = if is_custom {
            "custom-path".to_string()
        } else {
            item.group_id.clone()
        };
        out.push(scanner::ScanItem {
            id: format!(
                "analyzer::{}::{seq}",
                if is_custom { "custom" } else { &target_id }
            ),
            target_id,
            path: item.validated_item.path().to_string(),
            file_identity: None,
            size: item.size_bytes,
            mtime_unix_secs: 0,
            age_eligible: is_custom,
        });
    }
    Ok(out)
}

fn validate_node_convertible(node: &AnalyzerNode) -> Result<(), String> {
    if node.display_only {
        return Err(format!(
            "display-only node cannot be selected: {}",
            node.path
        ));
    }
    if !node.selectable {
        return Err(format!(
            "non-selectable node cannot be converted: {} ({})",
            node.path,
            node.selection_reason
                .clone()
                .unwrap_or_else(|| "not selectable".to_string())
        ));
    }
    if node.risk == Risk::Protected {
        return Err(format!("protected node cannot be converted: {}", node.path));
    }
    if node.is_link_point {
        return Err(format!(
            "link point node cannot be converted: {}",
            node.path
        ));
    }
    if node.is_cloud_placeholder {
        return Err(format!(
            "cloud placeholder node cannot be converted: {}",
            node.path
        ));
    }
    if annotate::is_vhdx_path(&node.path) {
        return Err(format!(".vhdx node cannot be converted: {}", node.path));
    }
    Ok(())
}

fn index_nodes(root: &AnalyzerNode) -> HashMap<String, AnalyzerNode> {
    fn rec(node: &AnalyzerNode, map: &mut HashMap<String, AnalyzerNode>) {
        map.insert(node.id.clone(), node.clone());
        for c in &node.children {
            rec(c, map);
        }
    }
    let mut map = HashMap::new();
    rec(root, &mut map);
    map
}
