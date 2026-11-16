use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use sweeper_contract::{
    ContractError, ExecuteCleanResponse, PreviewCleanResponse, PreviewConfirmationsDto,
    PreviewTargetBreakdownDto, SkippedReasonDto, U64String,
};
use sweeper_core::analyzer;
use sweeper_core::remover::{self, CleanOptions, CleanProgress, ValidatedItem};

use super::mapping::{
    analyzer_session_valid_at, risk_to_dto, sanitize_analyzer_build_msg, scan_session_valid_at,
};
use super::support::{
    detect_new_log_id, find_containing_root, hash_selection, list_log_ids, sanitize_reason,
};
use super::{
    AnalyzerSession, PreviewTokenSpec, PreviewTokenState, ScanSession, SessionStore, StoredItem,
    ValidatedCleanBatch, PREVIEW_TOKEN_TTL_SECS, SCAN_SESSION_TTL_SECS, TYPED_CONFIRMATION_PHRASE,
};
use crate::clock::Clock;

/// Cohesive request for previewing a clean selection.
#[derive(Debug)]
pub struct PreviewCleanRequest {
    /// Source scan/analyzer session id.
    pub session_id: String,
    /// Selected item IDs.
    pub item_ids: Vec<String>,
    /// Optional typed confirmations.
    pub confirmations: Option<PreviewConfirmationsDto>,
    /// Current settings generation for session validation.
    pub current_settings_generation: u64,
}

/// Cohesive request for executing a preview token.
#[derive(Debug)]
pub struct ExecuteCleanRequest {
    /// Preview token to consume and execute.
    pub preview_token: String,
    /// Current settings generation for validation.
    pub current_settings_generation: u64,
    /// Optional journal/log base directory.
    pub journal_base_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct PreviewRequest {
    session_id: String,
    item_ids: Vec<String>,
    confirmations: Option<PreviewConfirmationsDto>,
    settings_generation: u64,
}

#[derive(Debug)]
struct ScanPreviewBuild {
    total_bytes: u64,
    validated_items: Vec<ValidatedItem>,
    breakdown: Vec<PreviewTargetBreakdownDto>,
}

#[derive(Debug)]
struct AnalyzerPreviewBuild {
    total_bytes: u64,
    validated_items: Vec<ValidatedItem>,
    breakdown: Vec<PreviewTargetBreakdownDto>,
}

struct ScanAssembly<'a> {
    by_target: &'a mut BTreeMap<String, (String, u64, u64)>,
    total_bytes: &'a mut u64,
    validated: &'a mut Vec<ValidatedItem>,
}

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
        let session = self
            .get_valid_scan_session(clock, &req.session_id, req.settings_generation)?
            .clone();
        let build = Self::build_scan_preview(&session, &req)?;
        self.issue_preview_token(PreviewTokenSpec {
            now,
            session_id: req.session_id,
            item_ids: req.item_ids,
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
        let session = self
            .get_valid_analyzer_session(clock, &req.session_id, req.settings_generation)?
            .clone();
        let build = Self::build_analyzer_preview(&session, &req)?;
        self.issue_preview_token(PreviewTokenSpec {
            now,
            session_id: req.session_id,
            item_ids: req.item_ids,
            validated_items: build.validated_items,
            total_bytes: build.total_bytes,
            breakdown: build.breakdown,
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
        let selection_hash = hash_selection(&spec.session_id, &spec.item_ids);
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

    /// Validate and atomically consume a preview token for execution.
    pub fn consume_preview_token_for_execute<C: Clock>(
        &mut self,
        clock: &C,
        preview_token: &str,
        current_settings_generation: u64,
    ) -> Result<(u32, Vec<ValidatedItem>, u64), ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        let snapshot = self.snapshot_preview_token(preview_token)?;
        Self::validate_preview_snapshot(now, &snapshot)?;
        let min_age =
            self.validate_preview_session(clock, &snapshot, current_settings_generation)?;
        self.mark_preview_used(preview_token)?;
        Ok((
            min_age,
            snapshot.validated_items,
            snapshot.preview_estimated_bytes,
        ))
    }

    fn snapshot_preview_token(
        &self,
        preview_token: &str,
    ) -> Result<PreviewTokenState, ContractError> {
        self.preview_tokens
            .get(preview_token)
            .cloned()
            .ok_or_else(|| ContractError::PreviewTokenInvalid {
                reason: "token not found".to_string(),
            })
    }

    fn validate_preview_snapshot(now: u64, t: &PreviewTokenState) -> Result<(), ContractError> {
        if t.used {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token already used".to_string(),
            });
        }
        if now.saturating_sub(t.issued_at) > PREVIEW_TOKEN_TTL_SECS {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token expired".to_string(),
            });
        }
        let expected = hash_selection(&t.session_id, &t.item_ids);
        if expected != t.selection_hash {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token tampered".to_string(),
            });
        }
        Ok(())
    }

    fn validate_preview_session<C: Clock>(
        &self,
        clock: &C,
        t: &PreviewTokenState,
        settings_gen: u64,
    ) -> Result<u32, ContractError> {
        let now = clock.now_unix_secs();
        let (created_at, min_age) =
            self.get_valid_session_meta(clock, &t.session_id, settings_gen)?;
        if now.saturating_sub(created_at) > SCAN_SESSION_TTL_SECS {
            return Err(ContractError::SessionInvalid {
                reason: "session expired".to_string(),
            });
        }
        Ok(min_age)
    }

    fn mark_preview_used(&mut self, preview_token: &str) -> Result<(), ContractError> {
        let t = self.preview_tokens.get_mut(preview_token).ok_or_else(|| {
            ContractError::PreviewTokenInvalid {
                reason: "token not found".to_string(),
            }
        })?;
        if t.used {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token already used".to_string(),
            });
        }
        t.used = true;
        Ok(())
    }

    /// Execute clean for a preview token (consumes token atomically; runs removal while holding `&mut self`).
    ///
    /// Prefer `commands_impl::execute_clean` for lock-split execution.
    pub fn execute_clean<C, F>(
        &mut self,
        clock: &C,
        req: ExecuteCleanRequest,
        on_progress: F,
    ) -> Result<ExecuteCleanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        let (min_age_hours, validated_items, preview_estimated_bytes) = self
            .consume_preview_token_for_execute(
                clock,
                &req.preview_token,
                req.current_settings_generation,
            )?;
        Self::execute_validated_clean_batch(
            clock,
            ValidatedCleanBatch {
                preview_token: req.preview_token,
                min_age_hours,
                validated_items,
                preview_estimated_bytes,
                journal_base_dir: req.journal_base_dir,
            },
            on_progress,
        )
    }

    /// Run clean removal for an already-validated, token-consumed batch (no SessionStore lock required).
    pub fn execute_validated_clean_batch<C, F>(
        clock: &C,
        batch: ValidatedCleanBatch,
        mut on_progress: F,
    ) -> Result<ExecuteCleanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        let opts = Self::build_clean_options(clock, &batch);
        let resolved_log_dir = Self::resolved_log_dir(&opts.journal_base_dir);
        let existing_logs = list_log_ids(&resolved_log_dir);
        let report = Self::run_clean(
            batch.preview_token.clone(),
            batch.validated_items,
            opts,
            &mut on_progress,
        )?;
        let log_id = detect_new_log_id(&resolved_log_dir, &existing_logs);
        Ok(Self::summarize_clean(report, log_id))
    }

    fn build_clean_options<C: Clock>(clock: &C, batch: &ValidatedCleanBatch) -> CleanOptions {
        let now = UNIX_EPOCH + Duration::from_secs(clock.now_unix_secs());
        CleanOptions {
            preview_estimated_bytes: batch.preview_estimated_bytes,
            min_age: Duration::from_secs(u64::from(batch.min_age_hours) * 3600),
            now,
            journal_base_dir: batch.journal_base_dir.clone(),
        }
    }

    fn resolved_log_dir(journal_base_dir: &Option<PathBuf>) -> PathBuf {
        let log_dir = journal_base_dir
            .clone()
            .unwrap_or_else(sweeper_core::journal::default_logs_base_dir);
        sweeper_core::journal::resolve_log_dir(&log_dir)
    }

    fn run_clean<F>(
        preview_token: String,
        validated_items: Vec<ValidatedItem>,
        opts: CleanOptions,
        on_progress: &mut F,
    ) -> Result<sweeper_core::remover::CleanReport, ContractError>
    where
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        remover::execute_clean_with_progress(validated_items, opts, |p: CleanProgress| {
            on_progress(sweeper_contract::CleanProgressEvent {
                preview_token: preview_token.clone(),
                target_id: p.target_id,
                completed_items: U64String(p.completed_items),
                total_items: U64String(p.total_items),
                bytes_freed_so_far: U64String(p.bytes_freed_so_far),
                status: p.status,
            });
        })
        .map_err(|e| ContractError::Internal {
            reason: sanitize_reason(&e.to_string()),
        })
    }

    fn summarize_clean(
        report: sweeper_core::remover::CleanReport,
        log_id: Option<String>,
    ) -> ExecuteCleanResponse {
        let deleted_count = report
            .items
            .iter()
            .filter(|i| i.status == "deleted")
            .count() as u64;
        let skipped_reasons = Self::collect_skipped_reasons(&report.items);
        ExecuteCleanResponse {
            bytes_freed: U64String(report.bytes_freed),
            deleted_count: U64String(deleted_count),
            skipped_count: U64String(report.skipped_count),
            skipped_reasons,
            aborted: report.aborted,
            abort_reason: report.abort_reason,
            log_id,
        }
    }

    fn collect_skipped_reasons(
        items: &[sweeper_core::remover::ItemCleanResult],
    ) -> Vec<SkippedReasonDto> {
        let mut reasons: BTreeMap<String, u64> = BTreeMap::new();
        for item in items {
            if item.status == "Skipped" || item.status == "aborted" {
                let key = if item.reason.trim().is_empty() {
                    "Skipped".to_string()
                } else {
                    item.reason.clone()
                };
                *reasons.entry(key).or_insert(0) += 1;
            }
        }
        reasons
            .into_iter()
            .map(|(reason, count)| SkippedReasonDto {
                reason,
                count: U64String(count),
            })
            .collect()
    }

    /// Explain an item by id (scan item id, analyzer node id, or target/knowledge id).
    pub fn explain_item<C: Clock>(
        &mut self,
        clock: &C,
        item_id: &str,
        current_settings_generation: u64,
    ) -> Result<sweeper_contract::ExplainItemResponse, ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        self.explain_from_scan(now, item_id, current_settings_generation)
            .or_else(|| self.explain_from_analyzer(now, item_id, current_settings_generation))
            .or_else(|| Self::explain_from_knowledge(item_id))
            .ok_or_else(|| ContractError::NotFound {
                reason: "item not found".to_string(),
            })
    }

    fn explain_from_scan(
        &self,
        now: u64,
        item_id: &str,
        settings_gen: u64,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        for s in self.sessions.values() {
            if !scan_session_valid_at(s, now, settings_gen) {
                continue;
            }
            if let Some(resp) = Self::explain_scan_hit(s, item_id) {
                return Some(resp);
            }
        }
        None
    }

    fn explain_scan_hit(
        session: &ScanSession,
        item_id: &str,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        if let Some(item) = session.items.get(item_id) {
            return Some(sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title: item.target_name.clone(),
                description: analyzer::target_description(&item.target_id)
                    .unwrap_or_else(|| "Scanned item.".to_string()),
                risk: risk_to_dto(item.risk),
            });
        }
        session
            .results
            .targets
            .iter()
            .find(|t| t.target_id == item_id)
            .map(|t| sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title: t.target_name.clone(),
                description: analyzer::target_description(item_id)
                    .unwrap_or_else(|| "Target.".to_string()),
                risk: t.risk,
            })
    }

    fn explain_from_analyzer(
        &self,
        now: u64,
        item_id: &str,
        settings_gen: u64,
    ) -> Option<sweeper_contract::ExplainItemResponse> {
        for s in self.analyzer_sessions.values() {
            if !analyzer_session_valid_at(s, now, settings_gen) {
                continue;
            }
            if let Some(node) = analyzer::find_analyzer_node(&s.result.root, item_id) {
                let title = if node.name.is_empty() {
                    node.path.clone()
                } else {
                    node.name.clone()
                };
                return Some(sweeper_contract::ExplainItemResponse {
                    item_id: item_id.to_string(),
                    title,
                    description: node.description.clone(),
                    risk: risk_to_dto(node.risk),
                });
            }
        }
        None
    }

    fn explain_from_knowledge(item_id: &str) -> Option<sweeper_contract::ExplainItemResponse> {
        analyzer::knowledge_explanation(item_id).map(|(title, description, risk)| {
            sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title,
                description,
                risk: risk_to_dto(risk),
            }
        })
    }

    pub(super) fn get_valid_scan_session<C: Clock>(
        &self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<&ScanSession, ContractError> {
        let now = clock.now_unix_secs();
        let session =
            self.sessions
                .get(session_id)
                .ok_or_else(|| ContractError::SessionInvalid {
                    reason: "session not found".to_string(),
                })?;
        if !scan_session_valid_at(session, now, current_settings_generation) {
            return Err(ContractError::SessionInvalid {
                reason: "session expired or invalidated".to_string(),
            });
        }
        Ok(session)
    }

    pub(super) fn get_valid_analyzer_session<C: Clock>(
        &self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<&AnalyzerSession, ContractError> {
        let now = clock.now_unix_secs();
        let session = self.analyzer_sessions.get(session_id).ok_or_else(|| {
            ContractError::SessionInvalid {
                reason: "session not found".to_string(),
            }
        })?;
        if !analyzer_session_valid_at(session, now, current_settings_generation) {
            return Err(ContractError::SessionInvalid {
                reason: "session expired or invalidated".to_string(),
            });
        }
        Ok(session)
    }

    pub(super) fn get_valid_session_meta<C: Clock>(
        &self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<(u64, u32), ContractError> {
        let now = clock.now_unix_secs();
        if let Some(s) = self.sessions.get(session_id) {
            if !scan_session_valid_at(s, now, current_settings_generation) {
                return Err(ContractError::SessionInvalid {
                    reason: "session expired or invalidated".to_string(),
                });
            }
            return Ok((s.created_at, s.min_age_hours));
        }
        if let Some(s) = self.analyzer_sessions.get(session_id) {
            if !analyzer_session_valid_at(s, now, current_settings_generation) {
                return Err(ContractError::SessionInvalid {
                    reason: "session expired or invalidated".to_string(),
                });
            }
            return Ok((s.created_at, s.min_age_hours));
        }
        Err(ContractError::SessionInvalid {
            reason: "session not found".to_string(),
        })
    }
}
