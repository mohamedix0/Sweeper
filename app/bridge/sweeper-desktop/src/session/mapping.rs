use std::collections::HashMap;

use sweeper_contract::{
    AnalyzerNodeDto, ContractError, RiskDto, ScanItemDto, ScanResultsDto, StartAnalyzerResponse,
    TargetScanResultDto, TierDto, U64String,
};
use sweeper_core::analyzer::{self, AnalyzerNode, AnalyzerResult};
use sweeper_core::scanner::ScanReport;
use sweeper_core::{Risk, Tier};

use super::support::sanitize_reason;
use super::{AnalyzerSession, ScanSession, StoredItem};

pub(crate) fn scan_session_valid_at(
    session: &ScanSession,
    now: u64,
    current_settings_generation: u64,
) -> bool {
    if session.settings_generation != current_settings_generation {
        return false;
    }
    now.saturating_sub(session.created_at) <= super::SCAN_SESSION_TTL_SECS
}

pub(crate) fn analyzer_session_valid_at(
    session: &AnalyzerSession,
    now: u64,
    current_settings_generation: u64,
) -> bool {
    if session.settings_generation != current_settings_generation {
        return false;
    }
    now.saturating_sub(session.created_at) <= super::SCAN_SESSION_TTL_SECS
}

fn is_custom_path_scan_target(target_id: &str) -> bool {
    target_id == "custom-path" || target_id.starts_with("custom-path-")
}

fn tier_to_dto(t: Tier) -> TierDto {
    match t {
        Tier::Tier1 => TierDto::Tier1,
        Tier::Tier2 => TierDto::Tier2,
        Tier::Tier3 => TierDto::Tier3,
    }
}

fn option_tier_to_dto(t: Option<Tier>) -> Option<TierDto> {
    t.map(tier_to_dto)
}

pub(crate) fn map_analyzer_error_pub(e: analyzer::AnalyzerError) -> ContractError {
    map_analyzer_error(e)
}

pub(crate) fn map_analyzer_error(e: analyzer::AnalyzerError) -> ContractError {
    match e {
        analyzer::AnalyzerError::InvalidRoot(reason) | analyzer::AnalyzerError::Walk(reason) => {
            ContractError::InvalidInput { reason }
        }
        analyzer::AnalyzerError::NotFound(reason) => ContractError::NotFound { reason },
        analyzer::AnalyzerError::NotADirectory(reason) => ContractError::InvalidInput { reason },
    }
}

pub(crate) fn map_analyzer_result_to_dto(
    session_id: &str,
    result: &AnalyzerResult,
) -> StartAnalyzerResponse {
    StartAnalyzerResponse {
        session_id: session_id.to_string(),
        root_path: result.root.path.clone(),
        total_bytes: U64String(result.root.size_bytes),
        total_items: U64String(result.root.item_count),
        scanned_dirs: U64String(result.progress.scanned_dirs),
        scanned_files: U64String(result.progress.scanned_files),
        cancelled: result.cancelled,
        warnings: Vec::new(),
        root: map_analyzer_node_to_dto(&result.root),
    }
}

pub(crate) fn map_analyzer_node_to_dto(node: &AnalyzerNode) -> AnalyzerNodeDto {
    AnalyzerNodeDto {
        id: node.id.clone(),
        name: node.name.clone(),
        path: node.path.clone(),
        is_dir: node.is_dir,
        size_bytes: U64String(node.size_bytes),
        item_count: U64String(node.item_count),
        risk: risk_to_dto(node.risk),
        description: node.description.clone(),
        reason: node.selection_reason.clone(),
        tier: option_tier_to_dto(node.tier),
        target_id: node.target_id.clone(),
        is_link_point: node.is_link_point,
        is_cloud_placeholder: node.is_cloud_placeholder,
        display_only: node.display_only,
        selectable: node.selectable,
        children: node.children.iter().map(map_analyzer_node_to_dto).collect(),
    }
}

pub(crate) fn risk_to_dto(r: Risk) -> RiskDto {
    match r {
        Risk::Safe => RiskDto::Safe,
        Risk::Caution => RiskDto::Caution,
        Risk::Unknown => RiskDto::Unknown,
        Risk::Protected => RiskDto::Protected,
    }
}

fn map_scan_item(
    i: sweeper_core::scanner::ScanItem,
    target: &TargetMeta,
    items_map: &mut HashMap<String, StoredItem>,
) -> ScanItemDto {
    let dto = ScanItemDto {
        id: i.id.clone(),
        target_id: i.target_id.clone(),
        path: i.path,
        size: U64String(i.size),
        mtime_unix_secs: U64String(i.mtime_unix_secs),
        age_eligible: i.age_eligible,
    };
    items_map.insert(
        dto.id.clone(),
        StoredItem {
            dto: dto.clone(),
            target_id: target.target_id.clone(),
            target_name: target.target_name.clone(),
            tier: target.tier,
            risk: target.risk,
            age_eligible: i.age_eligible,
            requires_typed_confirmation: is_custom_path_scan_target(&target.target_id),
        },
    );
    dto
}

fn map_target_result(
    t: sweeper_core::scanner::TargetScanResult,
    items_map: &mut HashMap<String, StoredItem>,
) -> (String, Vec<String>, TargetScanResultDto) {
    let target_meta = TargetMeta {
        target_id: t.target_id.clone(),
        target_name: t.target_name.clone(),
        tier: t.tier,
        risk: t.risk,
    };
    let dto_items = t
        .items
        .into_iter()
        .map(|i| map_scan_item(i, &target_meta, items_map))
        .collect::<Vec<_>>();
    let dto = TargetScanResultDto {
        target_id: t.target_id.clone(),
        target_name: t.target_name,
        tier: tier_to_dto(t.tier),
        risk: risk_to_dto(t.risk),
        resolved_paths: t.resolved_paths.clone(),
        scanned: t.scanned,
        status_reason: t.status_reason,
        total_bytes: U64String(t.total_bytes),
        eligible_bytes: U64String(t.eligible_bytes),
        file_count: U64String(t.file_count),
        eligible_count: U64String(t.eligible_count),
        skipped_count: U64String(t.skipped_count),
        items: dto_items,
    };
    (t.target_id, t.resolved_paths, dto)
}

#[derive(Clone)]
struct TargetMeta {
    target_id: String,
    target_name: String,
    tier: Tier,
    risk: Risk,
}

pub(crate) fn map_report_to_session(
    session_id: &str,
    report: ScanReport,
) -> (
    ScanResultsDto,
    HashMap<String, StoredItem>,
    HashMap<String, Vec<String>>,
) {
    let mut items_map = HashMap::new();
    let mut target_roots = HashMap::new();
    let targets = report
        .targets
        .into_iter()
        .map(|t| {
            let (id, roots, dto) = map_target_result(t, &mut items_map);
            target_roots.insert(id, roots);
            dto
        })
        .collect::<Vec<_>>();
    let results = ScanResultsDto {
        session_id: session_id.to_string(),
        started_unix_secs: U64String(report.started_unix_secs),
        min_age_hours: report.min_age_hours,
        targets,
    };
    (results, items_map, target_roots)
}

pub(crate) fn sanitize_analyzer_build_msg(msg: &str) -> ContractError {
    if msg.contains("selected analyzer node not found") {
        ContractError::UnknownItem {
            count: U64String(1),
        }
    } else {
        ContractError::InvalidInput {
            reason: sanitize_reason(msg),
        }
    }
}
