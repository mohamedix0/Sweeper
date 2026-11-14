//! In-memory scan sessions and preview tokens with TTL enforcement.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use sweeper_contract::{
    AnalyzerNodeDto, AnalyzerProgressEvent, ContractError, ExecuteCleanResponse,
    PreviewCleanResponse, PreviewConfirmationsDto, PreviewTargetBreakdownDto, RiskDto, ScanItemDto,
    ScanResultsDto, SkippedReasonDto, StartAnalyzerResponse, StartScanResponse,
    TargetScanResultDto, TierDto, U64String,
};
use sweeper_core::analyzer::{
    self, AnalyzerNode, AnalyzerOptions, AnalyzerProgress, AnalyzerResult,
};
use sweeper_core::paths::CanonicalPath;
use sweeper_core::remover::{self, CleanOptions, CleanProgress, ValidatedItem};
use sweeper_core::scanner::{self, ScanReport};
use sweeper_core::settings::SweeperSettings;
use sweeper_core::{Risk, Tier};

use crate::clock::Clock;

const SCAN_SESSION_TTL_SECS: u64 = 15 * 60;
const PREVIEW_TOKEN_TTL_SECS: u64 = 10 * 60;
const TYPED_CONFIRMATION_PHRASE: &str = "DELETE";

/// Stored scan item with metadata for preview/validation.
#[derive(Debug, Clone)]
pub struct StoredItem {
    /// Item DTO for display.
    pub dto: ScanItemDto,
    /// Owning target id.
    pub target_id: String,
    /// Target name.
    pub target_name: String,
    /// Target tier.
    pub tier: Tier,
    /// Target risk.
    pub risk: Risk,
    /// Age eligible flag.
    pub age_eligible: bool,
    /// Is this a custom-path/unknown-like selection requiring typed confirmation?
    pub requires_typed_confirmation: bool,
}

/// Scan session state.
#[derive(Debug, Clone)]
pub struct ScanSession {
    /// Session id.
    pub id: String,
    /// Creation time (unix secs).
    pub created_at: u64,
    /// Settings generation at creation (invalidate if changed).
    pub settings_generation: u64,
    /// Scan results DTO.
    pub results: ScanResultsDto,
    /// Items by id.
    pub items: HashMap<String, StoredItem>,
    /// Resolved target roots by target id (for building validated items).
    pub target_roots: HashMap<String, Vec<String>>,
    /// Minimum age hours applied for this scan.
    pub min_age_hours: u32,
}

/// Analyzer session state.
#[derive(Debug, Clone)]
pub struct AnalyzerSession {
    /// Session id.
    pub id: String,
    /// Creation time (unix secs).
    pub created_at: u64,
    /// Settings generation at creation (invalidate if changed).
    pub settings_generation: u64,
    /// Analyzer result (tree + progress metadata).
    pub result: AnalyzerResult,
    /// Minimum age hours at time of analysis (for clean options).
    pub min_age_hours: u32,
}

/// Preview token state.
#[derive(Debug, Clone)]
pub struct PreviewTokenState {
    /// Token string.
    pub token: String,
    /// Session id.
    pub session_id: String,
    /// Issued time.
    pub issued_at: u64,
    /// Bound hash of selected item IDs.
    pub selection_hash: String,
    /// Selected item IDs.
    pub item_ids: Vec<String>,
    /// Validated items for clean execution.
    pub validated_items: Vec<ValidatedItem>,
    /// Preview estimated bytes (sum of selected item sizes).
    pub preview_estimated_bytes: u64,
    /// Whether token has been used/consumed.
    pub used: bool,
}

/// In-memory session store.
#[derive(Debug, Default)]
pub struct SessionStore {
    sessions: HashMap<String, ScanSession>,
    analyzer_sessions: HashMap<String, AnalyzerSession>,
    preview_tokens: HashMap<String, PreviewTokenState>,
    counter: u64,
}

impl SessionStore {
    /// New empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove expired sessions/tokens.
    pub fn cleanup<C: Clock>(&mut self, clock: &C) {
        let now = clock.now_unix_secs();
        self.sessions
            .retain(|_, s| now.saturating_sub(s.created_at) <= SCAN_SESSION_TTL_SECS);
        self.analyzer_sessions
            .retain(|_, s| now.saturating_sub(s.created_at) <= SCAN_SESSION_TTL_SECS);
        let valid_sessions: HashSet<String> = self
            .sessions
            .keys()
            .cloned()
            .chain(self.analyzer_sessions.keys().cloned())
            .collect();
        self.preview_tokens.retain(|_, t| {
            if !valid_sessions.contains(&t.session_id) {
                return false;
            }
            let age_ok = now.saturating_sub(t.issued_at) <= PREVIEW_TOKEN_TTL_SECS;
            let within_session = self
                .sessions
                .get(&t.session_id)
                .map(|s| now.saturating_sub(s.created_at) <= SCAN_SESSION_TTL_SECS)
                .or_else(|| {
                    self.analyzer_sessions
                        .get(&t.session_id)
                        .map(|s| now.saturating_sub(s.created_at) <= SCAN_SESSION_TTL_SECS)
                })
                .unwrap_or(false);
            age_ok && within_session && !t.used
        });
    }

    fn next_id(&mut self, prefix: &str, now: u64) -> String {
        self.counter = self.counter.saturating_add(1);
        format!("{prefix}-{now}-{}", self.counter)
    }

    /// Insert a synthetic session for tests.
    pub fn insert_test_session(&mut self, session: ScanSession) {
        self.sessions.insert(session.id.clone(), session);
    }

    /// Explicitly invalidate all active scan/analyzer sessions and preview tokens.
    pub fn invalidate_all_sessions(&mut self) {
        self.sessions.clear();
        self.analyzer_sessions.clear();
        self.preview_tokens.clear();
    }

    /// Direct access helper for tests: create a basic session with provided items/results.
    pub fn create_test_session_with_items(
        &mut self,
        now: u64,
        settings_generation: u64,
        session_id: &str,
        results: ScanResultsDto,
        items: HashMap<String, StoredItem>,
    ) {
        let target_roots = results
            .targets
            .iter()
            .map(|t| (t.target_id.clone(), t.resolved_paths.clone()))
            .collect();
        self.sessions.insert(
            session_id.to_string(),
            ScanSession {
                id: session_id.to_string(),
                created_at: now,
                settings_generation,
                results,
                items,
                target_roots,
                min_age_hours: 24,
            },
        );
    }

    /// Direct access helper for integration tests: insert a synthetic analyzer session.
    #[doc(hidden)]
    pub fn insert_test_analyzer_session(&mut self, session: AnalyzerSession) {
        self.analyzer_sessions.insert(session.id.clone(), session);
    }

    /// Create and store a scan session by running a read-only scan.
    pub fn start_scan<C: Clock>(
        &mut self,
        clock: &C,
        tiers: Option<Vec<TierDto>>,
        settings: &SweeperSettings,
        settings_generation: u64,
    ) -> Result<StartScanResponse, ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();

        let mut effective = settings.clone();
        if let Some(req) = tiers.as_deref().filter(|t| !t.is_empty()) {
            let req_set: HashSet<TierDto> = req.iter().copied().collect();
            effective.enable_tier1 = effective.enable_tier1 && req_set.contains(&TierDto::Tier1);
            effective.enable_tier2 = effective.enable_tier2 && req_set.contains(&TierDto::Tier2);
            effective.enable_tier3 = effective.enable_tier3 && req_set.contains(&TierDto::Tier3);
        }

        let report =
            scanner::scan_with_settings(&effective).map_err(|e| ContractError::Internal {
                reason: sanitize_reason(&e.to_string()),
            })?;

        let session_id = self.next_id("scan", now);
        let (results, items, target_roots) = map_report_to_session(&session_id, report);
        let session = ScanSession {
            id: session_id.clone(),
            created_at: now,
            settings_generation,
            results,
            items,
            target_roots,
            min_age_hours: effective.min_age_hours,
        };
        self.sessions.insert(session_id.clone(), session);
        Ok(StartScanResponse { session_id })
    }

    /// Get scan results for a valid scan session.
    pub fn get_scan_results<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<ScanResultsDto, ContractError> {
        self.cleanup(clock);
        let session =
            self.get_valid_scan_session(clock, session_id, current_settings_generation)?;
        Ok(session.results.clone())
    }

    /// Start a read-only analyzer session and store it for Preview/Clean routing.
    pub fn start_analyzer<C, P>(
        &mut self,
        clock: &C,
        root: Option<String>,
        settings: &SweeperSettings,
        settings_generation: u64,
        progress_cb: Option<P>,
    ) -> Result<StartAnalyzerResponse, ContractError>
    where
        C: Clock,
        P: Fn(AnalyzerProgressEvent) + Send + Sync + 'static,
    {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        let session_id = self.next_id("analyzer", now);
        let cancel = Arc::new(AtomicBool::new(false));
        let sid_for_progress = session_id.clone();
        let progress = progress_cb.map(|cb| {
            Arc::new(move |p: AnalyzerProgress| {
                cb(AnalyzerProgressEvent {
                    session_id: sid_for_progress.clone(),
                    scanned_dirs: U64String(p.scanned_dirs),
                    scanned_files: U64String(p.scanned_files),
                    scanned_bytes: U64String(p.scanned_bytes),
                    current_path: p.current_path,
                });
            }) as Arc<dyn Fn(AnalyzerProgress) + Send + Sync>
        });
        let options = AnalyzerOptions {
            allow_unknown: settings.allow_delete_unknown,
            ..AnalyzerOptions::default()
        };
        let result = analyzer::analyze_path(root.as_deref(), cancel, progress, options)
            .map_err(map_analyzer_error)?;
        let dto = map_analyzer_result_to_dto(&session_id, &result);
        self.analyzer_sessions.insert(
            session_id,
            AnalyzerSession {
                id: dto.session_id.clone(),
                created_at: now,
                settings_generation,
                result,
                min_age_hours: settings.min_age_hours,
            },
        );
        Ok(dto)
    }

    /// Create a preview token for selected item IDs (no clean).
    pub fn preview_clean<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        item_ids: Vec<String>,
        confirmations: Option<PreviewConfirmationsDto>,
        current_settings_generation: u64,
    ) -> Result<PreviewCleanResponse, ContractError> {
        self.cleanup(clock);
        if item_ids.is_empty() {
            return Err(ContractError::InvalidInput {
                reason: "no items selected".to_string(),
            });
        }
        if self.sessions.contains_key(session_id) {
            return self.preview_clean_scan_session(
                clock,
                session_id,
                item_ids,
                confirmations,
                current_settings_generation,
            );
        }
        if self.analyzer_sessions.contains_key(session_id) {
            return self.preview_clean_analyzer_session(
                clock,
                session_id,
                item_ids,
                confirmations,
                current_settings_generation,
            );
        }
        Err(ContractError::SessionInvalid {
            reason: "session not found".to_string(),
        })
    }

    fn preview_clean_scan_session<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        item_ids: Vec<String>,
        confirmations: Option<PreviewConfirmationsDto>,
        current_settings_generation: u64,
    ) -> Result<PreviewCleanResponse, ContractError> {
        let now = clock.now_unix_secs();
        let session = self
            .get_valid_scan_session(clock, session_id, current_settings_generation)?
            .clone();

        let mut selected: Vec<&StoredItem> = Vec::with_capacity(item_ids.len());
        let mut unknown = 0u64;
        for id in &item_ids {
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

        let needs_confirmation = selected.iter().any(|i| i.requires_typed_confirmation);
        if needs_confirmation {
            let ok = confirmations
                .as_ref()
                .and_then(|c| c.typed_confirmation.as_deref())
                == Some(TYPED_CONFIRMATION_PHRASE);
            if !ok {
                return Err(ContractError::InvalidInput {
                    reason: "typed confirmation required".to_string(),
                });
            }
        }

        let mut by_target: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
        let mut total_count = 0u64;
        let mut total_bytes = 0u64;
        let mut validated_items: Vec<ValidatedItem> = Vec::with_capacity(selected.len());

        for item in &selected {
            let entry = by_target
                .entry(item.target_id.clone())
                .or_insert_with(|| (item.target_name.clone(), 0u64, 0u64));
            entry.1 = entry.1.saturating_add(1);
            entry.2 = entry.2.saturating_add(item.dto.size.get());
            total_count += 1;
            total_bytes = total_bytes.saturating_add(item.dto.size.get());

            let roots = session
                .target_roots
                .get(&item.target_id)
                .cloned()
                .or_else(|| {
                    session
                        .results
                        .targets
                        .iter()
                        .find(|t| t.target_id == item.target_id)
                        .map(|t| t.resolved_paths.clone())
                })
                .unwrap_or_default();
            let target_root = find_containing_root(&roots, &item.dto.path).ok_or_else(|| {
                ContractError::InvalidInput {
                    reason: "selected item is not within a resolved target root".to_string(),
                }
            })?;
            let mtime = item.dto.mtime_unix_secs.get();
            let validated = ValidatedItem::from_allowlisted_scan(
                item.dto.id.clone(),
                item.target_id.clone(),
                target_root,
                item.dto.path.clone(),
                item.dto.size.get(),
                Some(mtime),
            )
            .map_err(|_| ContractError::InvalidInput {
                reason: "selected item failed validation".to_string(),
            })?;
            validated_items.push(validated);
        }

        let breakdown = by_target
            .into_iter()
            .map(
                |(target_id, (target_name, count, bytes))| PreviewTargetBreakdownDto {
                    target_id,
                    target_name,
                    selected_count: U64String(count),
                    selected_bytes: U64String(bytes),
                },
            )
            .collect::<Vec<_>>();

        self.issue_preview_token(
            now,
            session_id,
            &item_ids,
            validated_items,
            total_count,
            total_bytes,
            breakdown,
        )
    }

    fn preview_clean_analyzer_session<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        item_ids: Vec<String>,
        confirmations: Option<PreviewConfirmationsDto>,
        current_settings_generation: u64,
    ) -> Result<PreviewCleanResponse, ContractError> {
        let now = clock.now_unix_secs();
        let session = self
            .get_valid_analyzer_session(clock, session_id, current_settings_generation)?
            .clone();

        if item_ids.is_empty() {
            return Err(ContractError::InvalidInput {
                reason: "no items selected".to_string(),
            });
        }

        let mut unknown = 0u64;
        for id in &item_ids {
            if analyzer::find_analyzer_node(&session.result.root, id).is_none() {
                unknown += 1;
            }
        }
        if unknown > 0 {
            return Err(ContractError::UnknownItem {
                count: U64String(unknown),
            });
        }

        let preview_items = analyzer::build_analyzer_preview_items(&session.result, &item_ids)
            .map_err(|msg| {
                if msg.contains("selected analyzer node not found") {
                    ContractError::UnknownItem {
                        count: U64String(1),
                    }
                } else {
                    ContractError::InvalidInput {
                        reason: sanitize_reason(&msg),
                    }
                }
            })?;

        let needs_confirmation = preview_items.iter().any(|i| i.requires_typed_confirmation);
        if needs_confirmation {
            let ok = confirmations
                .as_ref()
                .and_then(|c| c.typed_confirmation.as_deref())
                == Some(TYPED_CONFIRMATION_PHRASE);
            if !ok {
                return Err(ContractError::InvalidInput {
                    reason: "typed confirmation required".to_string(),
                });
            }
        }

        let mut by_target: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
        let mut total_count = 0u64;
        let mut total_bytes = 0u64;
        let mut validated_items: Vec<ValidatedItem> = Vec::with_capacity(preview_items.len());

        for item in preview_items {
            let entry = by_target
                .entry(item.group_id.clone())
                .or_insert_with(|| (item.group_name.clone(), 0u64, 0u64));
            entry.1 = entry.1.saturating_add(1);
            entry.2 = entry.2.saturating_add(item.size_bytes);
            total_count += 1;
            total_bytes = total_bytes.saturating_add(item.size_bytes);
            validated_items.push(item.validated_item);
        }

        let breakdown = by_target
            .into_iter()
            .map(
                |(target_id, (target_name, count, bytes))| PreviewTargetBreakdownDto {
                    target_id,
                    target_name,
                    selected_count: U64String(count),
                    selected_bytes: U64String(bytes),
                },
            )
            .collect::<Vec<_>>();

        self.issue_preview_token(
            now,
            session_id,
            &item_ids,
            validated_items,
            total_count,
            total_bytes,
            breakdown,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn issue_preview_token(
        &mut self,
        now: u64,
        session_id: &str,
        item_ids: &[String],
        validated_items: Vec<ValidatedItem>,
        total_count: u64,
        total_bytes: u64,
        breakdown: Vec<PreviewTargetBreakdownDto>,
    ) -> Result<PreviewCleanResponse, ContractError> {
        let selection_hash = hash_selection(session_id, item_ids);
        let token = self.next_id("preview", now);
        let state = PreviewTokenState {
            token: token.clone(),
            session_id: session_id.to_string(),
            issued_at: now,
            selection_hash,
            item_ids: item_ids.to_vec(),
            validated_items,
            preview_estimated_bytes: total_bytes,
            used: false,
        };
        self.preview_tokens.insert(token.clone(), state);
        Ok(PreviewCleanResponse {
            preview_token: token,
            breakdown,
            total_selected_count: U64String(total_count),
            total_selected_bytes: U64String(total_bytes),
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
        let (
            session_id,
            item_ids,
            selection_hash,
            issued_at,
            used,
            validated_items,
            preview_estimated_bytes,
        ) = {
            let t = self.preview_tokens.get(preview_token).ok_or_else(|| {
                ContractError::PreviewTokenInvalid {
                    reason: "token not found".to_string(),
                }
            })?;
            (
                t.session_id.clone(),
                t.item_ids.clone(),
                t.selection_hash.clone(),
                t.issued_at,
                t.used,
                t.validated_items.clone(),
                t.preview_estimated_bytes,
            )
        };
        if used {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token already used".to_string(),
            });
        }
        let (created_at, min_age_hours) =
            self.get_valid_session_meta(clock, &session_id, current_settings_generation)?;
        if now.saturating_sub(issued_at) > PREVIEW_TOKEN_TTL_SECS {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token expired".to_string(),
            });
        }
        if now.saturating_sub(created_at) > SCAN_SESSION_TTL_SECS {
            return Err(ContractError::SessionInvalid {
                reason: "session expired".to_string(),
            });
        }
        let expected = hash_selection(&session_id, &item_ids);
        if expected != selection_hash {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token tampered".to_string(),
            });
        }
        if let Some(t) = self.preview_tokens.get_mut(preview_token) {
            if t.used {
                return Err(ContractError::PreviewTokenInvalid {
                    reason: "token already used".to_string(),
                });
            }
            t.used = true;
        }
        Ok((min_age_hours, validated_items, preview_estimated_bytes))
    }

    /// Execute clean for a preview token (consumes token atomically).
    pub fn execute_clean<C, F>(
        &mut self,
        clock: &C,
        preview_token: &str,
        current_settings_generation: u64,
        journal_base_dir: Option<PathBuf>,
        mut on_progress: F,
    ) -> Result<ExecuteCleanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        let (min_age_hours, validated_items, preview_estimated_bytes) = self
            .consume_preview_token_for_execute(clock, preview_token, current_settings_generation)?;

        let _total_items = validated_items.len() as u64;
        let now = UNIX_EPOCH + Duration::from_secs(clock.now_unix_secs());
        let opts = CleanOptions {
            preview_estimated_bytes,
            min_age: Duration::from_secs(u64::from(min_age_hours) * 3600),
            now,
            journal_base_dir: journal_base_dir.clone(),
        };

        let log_dir = opts
            .journal_base_dir
            .clone()
            .unwrap_or_else(sweeper_core::journal::default_logs_base_dir);
        let resolved_log_dir = sweeper_core::journal::resolve_log_dir(&log_dir);
        let existing_logs = list_log_ids(&resolved_log_dir);

        let report =
            remover::execute_clean_with_progress(validated_items, opts, |p: CleanProgress| {
                on_progress(sweeper_contract::CleanProgressEvent {
                    preview_token: preview_token.to_string(),
                    target_id: p.target_id,
                    completed_items: U64String(p.completed_items),
                    total_items: U64String(p.total_items),
                    bytes_freed_so_far: U64String(p.bytes_freed_so_far),
                    status: p.status,
                });
            })
            .map_err(|e| ContractError::Internal {
                reason: sanitize_reason(&e.to_string()),
            })?;

        let log_id = detect_new_log_id(&resolved_log_dir, &existing_logs);

        let deleted_count = report
            .items
            .iter()
            .filter(|i| i.status == "deleted")
            .count() as u64;
        let mut reasons: BTreeMap<String, u64> = BTreeMap::new();
        for item in &report.items {
            if item.status == "Skipped" || item.status == "aborted" {
                let key = if item.reason.trim().is_empty() {
                    "Skipped".to_string()
                } else {
                    item.reason.clone()
                };
                *reasons.entry(key).or_insert(0) += 1;
            }
        }
        let skipped_reasons = reasons
            .into_iter()
            .map(|(reason, count)| SkippedReasonDto {
                reason,
                count: U64String(count),
            })
            .collect::<Vec<_>>();

        Ok(ExecuteCleanResponse {
            bytes_freed: U64String(report.bytes_freed),
            deleted_count: U64String(deleted_count),
            skipped_count: U64String(report.skipped_count),
            skipped_reasons,
            aborted: report.aborted,
            abort_reason: report.abort_reason,
            log_id,
        })
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
        for s in self.sessions.values() {
            if !scan_session_valid_at(s, now, current_settings_generation) {
                continue;
            }
            if let Some(item) = s.items.get(item_id) {
                return Ok(sweeper_contract::ExplainItemResponse {
                    item_id: item_id.to_string(),
                    title: item.target_name.clone(),
                    description: analyzer::target_description(&item.target_id)
                        .unwrap_or_else(|| "Scanned item.".to_string()),
                    risk: risk_to_dto(item.risk),
                });
            }
            if let Some(t) = s.results.targets.iter().find(|t| t.target_id == item_id) {
                return Ok(sweeper_contract::ExplainItemResponse {
                    item_id: item_id.to_string(),
                    title: t.target_name.clone(),
                    description: analyzer::target_description(item_id)
                        .unwrap_or_else(|| "Target.".to_string()),
                    risk: t.risk,
                });
            }
        }
        for s in self.analyzer_sessions.values() {
            if !analyzer_session_valid_at(s, now, current_settings_generation) {
                continue;
            }
            if let Some(node) = analyzer::find_analyzer_node(&s.result.root, item_id) {
                let title = if node.name.is_empty() {
                    node.path.clone()
                } else {
                    node.name.clone()
                };
                return Ok(sweeper_contract::ExplainItemResponse {
                    item_id: item_id.to_string(),
                    title,
                    description: node.description.clone(),
                    risk: risk_to_dto(node.risk),
                });
            }
        }
        if let Some((title, description, risk)) = analyzer::knowledge_explanation(item_id) {
            return Ok(sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title,
                description,
                risk: risk_to_dto(risk),
            });
        }
        Err(ContractError::NotFound {
            reason: "item not found".to_string(),
        })
    }

    fn get_valid_scan_session<C: Clock>(
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

    fn get_valid_analyzer_session<C: Clock>(
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

    fn get_valid_session_meta<C: Clock>(
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

fn scan_session_valid_at(
    session: &ScanSession,
    now: u64,
    current_settings_generation: u64,
) -> bool {
    if session.settings_generation != current_settings_generation {
        return false;
    }
    now.saturating_sub(session.created_at) <= SCAN_SESSION_TTL_SECS
}

fn analyzer_session_valid_at(
    session: &AnalyzerSession,
    now: u64,
    current_settings_generation: u64,
) -> bool {
    if session.settings_generation != current_settings_generation {
        return false;
    }
    now.saturating_sub(session.created_at) <= SCAN_SESSION_TTL_SECS
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

fn map_analyzer_error(e: analyzer::AnalyzerError) -> ContractError {
    match e {
        analyzer::AnalyzerError::InvalidRoot(reason) | analyzer::AnalyzerError::Walk(reason) => {
            ContractError::InvalidInput { reason }
        }
        analyzer::AnalyzerError::NotFound(reason) => ContractError::NotFound { reason },
        analyzer::AnalyzerError::NotADirectory(reason) => ContractError::InvalidInput { reason },
    }
}

fn map_analyzer_result_to_dto(session_id: &str, result: &AnalyzerResult) -> StartAnalyzerResponse {
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

fn map_analyzer_node_to_dto(node: &AnalyzerNode) -> AnalyzerNodeDto {
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

fn risk_to_dto(r: Risk) -> RiskDto {
    match r {
        Risk::Safe => RiskDto::Safe,
        Risk::Caution => RiskDto::Caution,
        Risk::Unknown => RiskDto::Unknown,
        Risk::Protected => RiskDto::Protected,
    }
}

fn map_report_to_session(
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
            target_roots.insert(t.target_id.clone(), t.resolved_paths.clone());
            let dto_items = t
                .items
                .into_iter()
                .map(|i| {
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
                            target_id: t.target_id.clone(),
                            target_name: t.target_name.clone(),
                            tier: t.tier,
                            risk: t.risk,
                            age_eligible: i.age_eligible,
                            requires_typed_confirmation: is_custom_path_scan_target(&t.target_id),
                        },
                    );
                    dto
                })
                .collect::<Vec<_>>();
            TargetScanResultDto {
                target_id: t.target_id,
                target_name: t.target_name,
                tier: tier_to_dto(t.tier),
                risk: risk_to_dto(t.risk),
                resolved_paths: t.resolved_paths,
                scanned: t.scanned,
                status_reason: t.status_reason,
                total_bytes: U64String(t.total_bytes),
                eligible_bytes: U64String(t.eligible_bytes),
                file_count: U64String(t.file_count),
                eligible_count: U64String(t.eligible_count),
                skipped_count: U64String(t.skipped_count),
                items: dto_items,
            }
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

fn hash_selection(session_id: &str, item_ids: &[String]) -> String {
    let mut ids = item_ids.to_vec();
    ids.sort();
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update(b"\0");
    for id in ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\n");
    }
    let out = hasher.finalize();
    hex::encode(out)
}

fn find_containing_root(roots: &[String], item_path: &str) -> Option<String> {
    let item = CanonicalPath::parse_lexical(item_path).ok()?;
    let mut best: Option<(usize, String)> = None;
    for root_str in roots {
        let Ok(root) = CanonicalPath::parse_lexical(root_str) else {
            continue;
        };
        if root.is_self_or_ancestor_of(&item) && !root.is_same_path(&item) {
            let depth = root.segment_count();
            match &best {
                Some((best_depth, _)) if *best_depth >= depth => {}
                _ => best = Some((depth, root_str.clone())),
            }
        }
    }
    best.map(|(_, r)| r)
}

fn sanitize_reason(s: &str) -> String {
    let lower = s.to_lowercase();
    if lower.contains("path")
        || lower.contains('\\')
        || lower.contains('/')
        || lower.contains("stack")
    {
        "operation failed".to_string()
    } else {
        let mut out = s.trim().to_string();
        if out.len() > 120 {
            out.truncate(120);
        }
        if out.is_empty() {
            "operation failed".to_string()
        } else {
            out
        }
    }
}

fn list_log_ids(log_dir: &std::path::Path) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(log_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.insert(stem.to_string());
            }
        }
    }
    out
}

fn detect_new_log_id(log_dir: &std::path::Path, existing: &HashSet<String>) -> Option<String> {
    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(log_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if existing.contains(stem) {
                continue;
            }
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            candidates.push((modified, stem.to_string()));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_root_prefers_longest_match() {
        let roots = vec![
            r"C:\Users\A\AppData\Local".to_string(),
            r"C:\Users\A\AppData\Local\Temp".to_string(),
        ];
        let found = find_containing_root(&roots, r"C:\Users\A\AppData\Local\Temp\f.txt");
        assert_eq!(found.as_deref(), Some(r"C:\Users\A\AppData\Local\Temp"));
    }
}
