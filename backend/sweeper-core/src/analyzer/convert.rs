use super::*;
use crate::paths::CanonicalPath;
use crate::policy;
use crate::remover::ValidatedItem;
use crate::scanner;
use std::collections::{HashMap, HashSet};
use sweeper_knowledge::{embedded_targets, Risk};

/// Convert selected eligible Analyzer node IDs into validated scan items for Preview/Clean routing.
///
/// This seam performs explicit safety re-validation:
/// - selected nodes must exist in the provided tree
/// - nodes must be selectable and not display-only/protected/link points/cloud placeholders/`.vhdx`
/// - allowlist-backed nodes are mapped to allowlisted validated items
/// - only safe, non-protected, non-container, depth>=3, non-link-point directories outside protected
///   subtrees/containers are eligible as custom-path-root conversions (contents-only semantics)
pub fn build_scan_items_from_analyzer(
    result: &AnalyzerResult,
    selected_node_ids: &[String],
) -> Result<Vec<scanner::ScanItem>, String> {
    let index = index_nodes(&result.root);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut seq: u64 = 0;

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
            let _validated = ValidatedItem::from_allowlisted_scan(
                format!("analyzer::{tid}::{}", node.id),
                tid.clone(),
                target_root,
                node.path.clone(),
                node.size_bytes,
                None,
            )
            .map_err(|e| format!("selected node rejected by allowlist conversion: {e}"))?;

            seq += 1;
            out.push(scanner::ScanItem {
                id: format!("analyzer::{tid}::{seq}"),
                target_id: tid.clone(),
                path: node.path.clone(),
                file_identity: None,
                size: node.size_bytes,
                mtime_unix_secs: 0,
                age_eligible: false,
            });
            continue;
        }

        // Custom-path-root seam (Phase 5 will supply explicit typed confirmation flows at higher layers).
        // Restrict to directories and re-run on-disk custom path validation/policy checks.
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

        seq += 1;
        out.push(scanner::ScanItem {
            id: format!("analyzer::custom::{seq}"),
            target_id: "custom-path".to_string(),
            path: node.path.clone(),
            file_identity: None,
            size: node.size_bytes,
            mtime_unix_secs: 0,
            age_eligible: true,
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
