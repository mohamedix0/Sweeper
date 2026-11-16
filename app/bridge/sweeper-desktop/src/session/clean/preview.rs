use std::collections::BTreeMap;

use sweeper_contract::{
    ContractError, PreviewCleanResponse, PreviewConfirmationsDto, PreviewTargetBreakdownDto,
    U64String,
};
use sweeper_core::analyzer;
use sweeper_core::remover::ValidatedItem;

use super::super::mapping::sanitize_analyzer_build_msg;
use super::super::support::{find_containing_root, hash_selection, SessionIdRef};
use super::super::{
    AnalyzerSession, PreviewTokenSpec, PreviewTokenState, ScanSession, SessionStore, StoredItem,
    TYPED_CONFIRMATION_PHRASE,
};
use super::{
    AnalyzerPreviewBuild, PreviewCleanRequest, PreviewRequest, ScanAssembly, ScanPreviewBuild,
};

struct TokenIssueRequest {
    now: u64,
    req: PreviewRequest,
    validated_items: Vec<ValidatedItem>,
    total_bytes: u64,
    breakdown: Vec<PreviewTargetBreakdownDto>,
}
use crate::clock::Clock;

impl SessionStore {
    /// Create a preview token for selected item IDs (no clean).
    pub fn preview_clean<C: Clock>(
        &mut self,
        clock: &C,
        req: PreviewCleanRequest,
    ) -> Result<PreviewCleanResponse, ContractError> {
        self.cleanup(clock);
        let req = PreviewRequest {
            session_id: req.session_id,
            item_ids: req.item_ids,
            confirmations: req.confirmations,
            settings_generation: req.current_settings_generation,
        };
        self.dispatch_preview(clock, req)
    }

    fn dispatch_preview<C: Clock>(
        &mut self,
        clock: &C,
        req: PreviewRequest,
    ) -> Result<PreviewCleanResponse, ContractError> {
        if req.item_ids.is_empty() {
            return Err(ContractError::InvalidInput {
                reason: "no items selected".to_string(),
            });
        }
        if self.sessions.contains_key(&req.session_id) {
            return self.preview_scan(clock, req);
        }
        if self.analyzer_sessions.contains_key(&req.session_id) {
            return self.preview_analyzer(clock, req);
        }
        Err(ContractError::SessionInvalid {
            reason: "session not found".to_string(),
        })
    }

    fn preview_scan<C: Clock>(
        &mut self,
        clock: &C,
        req: PreviewRequest,
    ) -> Result<PreviewCleanResponse, ContractError> {
        let now = clock.now_unix_secs();
        let lookup = super::SessionLookup::new(&req.session_id, req.settings_generation);
        let session = self.get_valid_scan_session(clock, lookup)?.clone();
        let build = Self::build_scan_preview(&session, &req)?;
        self.issue_token_from(TokenIssueRequest {
            now,
            req,
            validated_items: build.validated_items,
            total_bytes: build.total_bytes,
            breakdown: build.breakdown,
        })
    }

    fn build_scan_preview(
        session: &ScanSession,
        req: &PreviewRequest,
    ) -> Result<ScanPreviewBuild, ContractError> {
        let selected = Self::collect_scan_items(session, &req.item_ids)?;
        Self::require_confirmation(selected.iter().copied(), &req.confirmations)?;
        Self::assemble_scan_items(session, &selected)
    }

    fn collect_scan_items<'a>(
        session: &'a ScanSession,
        item_ids: &[String],
    ) -> Result<Vec<&'a StoredItem>, ContractError> {
        let mut selected = Vec::with_capacity(item_ids.len());
        let mut unknown = 0u64;
        for id in item_ids {
            match session.items.get(id) {
                Some(item) => selected.push(item),
                None => unknown += 1,
            }
        }
        if unknown > 0 {
            return Err(ContractError::UnknownItem {
                count: U64String(unknown),
            });
        }
        Ok(selected)
    }

    fn require_confirmation<'a, I>(
        items: I,
        confirmations: &Option<PreviewConfirmationsDto>,
    ) -> Result<(), ContractError>
    where
        I: Iterator<Item = &'a StoredItem>,
    {
        let needed = items.into_iter().any(|i| i.requires_typed_confirmation);
        if !needed {
            return Ok(());
        }
        let ok = confirmations
            .as_ref()
            .and_then(|c| c.typed_confirmation.as_deref())
            == Some(TYPED_CONFIRMATION_PHRASE);
        if ok {
            Ok(())
        } else {
            Err(ContractError::InvalidInput {
                reason: "typed confirmation required".to_string(),
            })
        }
    }

    fn assemble_scan_items(
        session: &ScanSession,
        selected: &[&StoredItem],
    ) -> Result<ScanPreviewBuild, ContractError> {
        let mut by_target: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
        let mut total_bytes = 0u64;
        let mut validated = Vec::with_capacity(selected.len());
        for item in selected {
            let mut assembly = ScanAssembly {
                by_target: &mut by_target,
                total_bytes: &mut total_bytes,
                validated: &mut validated,
            };
            Self::accumulate_scan_item(session, item, &mut assembly)?;
        }
        Ok(ScanPreviewBuild {
            total_bytes,
            validated_items: validated,
            breakdown: Self::to_breakdown(by_target),
        })
    }

    fn accumulate_scan_item(
        session: &ScanSession,
        item: &StoredItem,
        assembly: &mut ScanAssembly<'_>,
    ) -> Result<(), ContractError> {
        let size = item.dto.size.get();
        let entry = assembly
            .by_target
            .entry(item.target_id.clone())
            .or_insert_with(|| (item.target_name.clone(), 0u64, 0u64));
        entry.1 = entry.1.saturating_add(1);
        entry.2 = entry.2.saturating_add(size);
        *assembly.total_bytes = assembly.total_bytes.saturating_add(size);
        assembly
            .validated
            .push(Self::validate_scan_item(session, item)?);
        Ok(())
    }

    fn validate_scan_item(
        session: &ScanSession,
        item: &StoredItem,
    ) -> Result<ValidatedItem, ContractError> {
        let roots = Self::scan_item_roots(session, &item.target_id);
        if let Some(target_root) = Self::find_same_scan_root(&roots, &item.dto.path) {
            return Self::validate_scan_directory_item(item, target_root);
        }
        let target_root = find_containing_root(&roots, &item.dto.path).ok_or_else(|| {
            ContractError::InvalidInput {
                reason: "selected item is not within a resolved target root".to_string(),
            }
        })?;
        ValidatedItem::from_allowlisted_scan(
            item.dto.id.clone(),
            item.target_id.clone(),
            target_root,
            item.dto.path.clone(),
            item.dto.size.get(),
            Some(item.dto.mtime_unix_secs.get()),
        )
        .map_err(|_| ContractError::InvalidInput {
            reason: "selected item failed validation".to_string(),
        })
    }

    fn find_same_scan_root(roots: &[String], item_path: &str) -> Option<String> {
        let item = sweeper_core::paths::CanonicalPath::parse_lexical(item_path).ok()?;
        for root_str in roots {
            let Ok(root) = sweeper_core::paths::CanonicalPath::parse_lexical(root_str) else {
                continue;
            };
            if root.is_same_path(&item) {
                return Some(root_str.clone());
            }
        }
        None
    }

    fn validate_scan_directory_item(
        item: &StoredItem,
        target_root: String,
    ) -> Result<ValidatedItem, ContractError> {
        if matches!(
            item.target_id.as_str(),
            "stale-rust-target" | "stale-node-modules"
        ) {
            return ValidatedItem::from_tier3_project_directory(
                item.dto.id.clone(),
                item.target_id.clone(),
                item.dto.path.clone(),
                item.dto.size.get(),
                Some(item.dto.mtime_unix_secs.get()),
            )
            .map_err(|_| ContractError::InvalidInput {
                reason: "selected item failed validation".to_string(),
            });
        }
        // Same-path non-stale items are not valid contents-only scan selections.
        let _ = target_root;
        Err(ContractError::InvalidInput {
            reason: "selected item is not within a resolved target root".to_string(),
        })
    }

    fn scan_item_roots(session: &ScanSession, target_id: &str) -> Vec<String> {
        session
            .target_roots
            .get(target_id)
            .cloned()
            .or_else(|| {
                session
                    .results
                    .targets
                    .iter()
                    .find(|t| t.target_id == target_id)
                    .map(|t| t.resolved_paths.clone())
            })
            .unwrap_or_default()
    }

    fn to_breakdown(
        by_target: BTreeMap<String, (String, u64, u64)>,
    ) -> Vec<PreviewTargetBreakdownDto> {
        by_target
            .into_iter()
            .map(
                |(target_id, (target_name, count, bytes))| PreviewTargetBreakdownDto {
                    target_id,
                    target_name,
                    selected_count: U64String(count),
                    selected_bytes: U64String(bytes),
                },
            )
            .collect()
    }

    fn preview_analyzer<C: Clock>(
        &mut self,
        clock: &C,
        req: PreviewRequest,
    ) -> Result<PreviewCleanResponse, ContractError> {
        let now = clock.now_unix_secs();
        let lookup = super::SessionLookup::new(&req.session_id, req.settings_generation);
        let session = self.get_valid_analyzer_session(clock, lookup)?.clone();
        let build = Self::build_analyzer_preview(&session, &req)?;
        self.issue_token_from(TokenIssueRequest {
            now,
            req,
            validated_items: build.validated_items,
            total_bytes: build.total_bytes,
            breakdown: build.breakdown,
        })
    }

    fn issue_token_from(
        &mut self,
        spec: TokenIssueRequest,
    ) -> Result<PreviewCleanResponse, ContractError> {
        self.issue_preview_token(PreviewTokenSpec {
            now: spec.now,
            session_id: spec.req.session_id,
            item_ids: spec.req.item_ids,
            validated_items: spec.validated_items,
            total_bytes: spec.total_bytes,
            breakdown: spec.breakdown,
        })
    }

    fn build_analyzer_preview(
        session: &AnalyzerSession,
        req: &PreviewRequest,
    ) -> Result<AnalyzerPreviewBuild, ContractError> {
        Self::ensure_analyzer_nodes_exist(session, &req.item_ids)?;
        let preview_items = analyzer::build_analyzer_preview_items(&session.result, &req.item_ids)
            .map_err(|msg| sanitize_analyzer_build_msg(&msg))?;
        Self::require_analyzer_confirmation(&preview_items, &req.confirmations)?;
        Ok(Self::assemble_analyzer_items(preview_items))
    }

    fn ensure_analyzer_nodes_exist(
        session: &AnalyzerSession,
        item_ids: &[String],
    ) -> Result<(), ContractError> {
        let mut unknown = 0u64;
        for id in item_ids {
            if analyzer::find_analyzer_node(&session.result.root, id).is_none() {
                unknown += 1;
            }
        }
        if unknown > 0 {
            return Err(ContractError::UnknownItem {
                count: U64String(unknown),
            });
        }
        Ok(())
    }

    fn require_analyzer_confirmation(
        items: &[analyzer::AnalyzerPreviewItem],
        confirmations: &Option<PreviewConfirmationsDto>,
    ) -> Result<(), ContractError> {
        let needed = items.iter().any(|i| i.requires_typed_confirmation);
        if !needed {
            return Ok(());
        }
        let ok = confirmations
            .as_ref()
            .and_then(|c| c.typed_confirmation.as_deref())
            == Some(TYPED_CONFIRMATION_PHRASE);
        if ok {
            Ok(())
        } else {
            Err(ContractError::InvalidInput {
                reason: "typed confirmation required".to_string(),
            })
        }
    }

    fn assemble_analyzer_items(items: Vec<analyzer::AnalyzerPreviewItem>) -> AnalyzerPreviewBuild {
        let mut by_target: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
        let mut total_bytes = 0u64;
        let mut validated = Vec::with_capacity(items.len());
        for item in items {
            let entry = by_target
                .entry(item.group_id.clone())
                .or_insert_with(|| (item.group_name.clone(), 0u64, 0u64));
            entry.1 = entry.1.saturating_add(1);
            entry.2 = entry.2.saturating_add(item.size_bytes);
            total_bytes = total_bytes.saturating_add(item.size_bytes);
            validated.push(item.validated_item);
        }
        AnalyzerPreviewBuild {
            total_bytes,
            validated_items: validated,
            breakdown: Self::to_breakdown(by_target),
        }
    }

    fn issue_preview_token(
        &mut self,
        spec: PreviewTokenSpec,
    ) -> Result<PreviewCleanResponse, ContractError> {
        let selection_hash = hash_selection(SessionIdRef(&spec.session_id), &spec.item_ids);
        let token = self.next_id("preview", spec.now);
        let total_selected_count = spec.validated_items.len() as u64;
        let state = PreviewTokenState {
            token: token.clone(),
            session_id: spec.session_id,
            issued_at: spec.now,
            selection_hash,
            item_ids: spec.item_ids,
            validated_items: spec.validated_items,
            preview_estimated_bytes: spec.total_bytes,
            used: false,
        };
        self.preview_tokens.insert(token.clone(), state);
        Ok(PreviewCleanResponse {
            preview_token: token,
            breakdown: spec.breakdown,
            total_selected_count: U64String(total_selected_count),
            total_selected_bytes: U64String(spec.total_bytes),
        })
    }
}
