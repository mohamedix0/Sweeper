use super::*;
use crate::paths::CanonicalPath;
use crate::policy;
use crate::remover::ValidatedItem;
use crate::scanner;
use std::collections::{HashMap, HashSet};
use sweeper_knowledge::{embedded_targets, Risk};

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

        if let Some(tid) = &node.target_id {
            let target = embedded_targets()
                .into_iter()
                .find(|t| &t.id == tid)
                .ok_or_else(|| format!("allowlist target missing for node {}: {tid}", node.id))?;
            let target_roots = annotate::resolve_target_roots_for_def(&target);
            let node_canon = CanonicalPath::parse_lexical(&node.path)
                .map_err(|e| format!("selected node path invalid for conversion: {e}"))?;
            let target_root = target_roots
                .iter()
                .filter(|r| r.is_self_or_ancestor_of(&node_canon) && !r.is_same_path(&node_canon))
                .max_by_key(|r| r.segment_count())
                .map(|r| r.to_string())
                .ok_or_else(|| {
                    format!(
                        "failed to resolve matching allowlist target root for {tid} and node {}",
                        node.path
                    )
                })?;
            let validated_item = ValidatedItem::from_allowlisted_scan(
                node.id.clone(),
                tid.clone(),
                target_root,
                node.path.clone(),
                node.size_bytes,
                None,
            )
            .map_err(|e| format!("selected node rejected by allowlist conversion: {e}"))?;

            out.push(AnalyzerPreviewItem {
                selected_node_id: node.id.clone(),
                validated_item,
                group_id: tid.clone(),
                group_name: if target.description.is_empty() {
                    target.name.clone()
                } else {
                    target.description.clone()
                },
                size_bytes: node.size_bytes,
                requires_typed_confirmation: false,
            });
            continue;
        }

        // Custom-path-root seam (contents-only semantics, typed confirmation required).
        // Eligible Unknown selections (opt-in) and other non-target custom-path selections
        // must route through `ValidatedItem::from_custom_path_root` and require typed confirmation.
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
        if matches!(node.risk, Risk::Protected)
            || annotate::is_vhdx_path(&node.path)
            || node.is_link_point
            || node.is_cloud_placeholder
        {
            return Err(format!(
                "selected custom-path candidate rejected by safety policy: {}",
                node.path
            ));
        }
        let validated_item = ValidatedItem::from_custom_path_root(
            node.id.clone(),
            node.path.clone(),
            node.size_bytes,
        )
        .map_err(|e| format!("selected custom-path candidate rejected: {e}"))?;

        out.push(AnalyzerPreviewItem {
            selected_node_id: node.id.clone(),
            validated_item,
            group_id: "custom-path".to_string(),
            group_name: "Custom Path".to_string(),
            size_bytes: node.size_bytes,
            requires_typed_confirmation: true,
        });
    }

    Ok(out)
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
