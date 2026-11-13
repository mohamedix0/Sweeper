//! In-memory scan sessions and preview tokens with TTL enforcement.

use std::collections::{BTreeMap, HashMap, HashSet};

use sha2::{Digest, Sha256};
use sweeper_contract::{
    ContractError, PreviewCleanResponse, PreviewConfirmationsDto, PreviewTargetBreakdownDto,
    RiskDto, ScanItemDto, ScanResultsDto, SettingsDto, StartScanResponse, TargetScanResultDto,
    TierDto, U64String,
};
use sweeper_core::scanner::{self, ScanReport};
use sweeper_knowledge::{Risk, Tier};

use crate::clock::Clock;

const SCAN_SESSION_TTL_SECS: u64 = 15 * 60;
const PREVIEW_TOKEN_TTL_SECS: u64 = 10 * 60;

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
    /// Is this a custom-path/unknown-like selection requiring typed confirmation? (Phase 2: allowlist items only)
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
    /// Whether token has been used.
    pub used: bool,
}

/// In-memory session store.
#[derive(Debug, Default)]
pub struct SessionStore {
    sessions: HashMap<String, ScanSession>,
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
        let valid_sessions: HashSet<String> = self.sessions.keys().cloned().collect();
        self.preview_tokens.retain(|_, t| {
            if !valid_sessions.contains(&t.session_id) {
                return false;
            }
            let age_ok = now.saturating_sub(t.issued_at) <= PREVIEW_TOKEN_TTL_SECS;
            let session = self.sessions.get(&t.session_id);
            let within_session = session
                .map(|s| now.saturating_sub(s.created_at) <= SCAN_SESSION_TTL_SECS)
                .unwrap_or(false);
            age_ok && within_session
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

    /// Direct access helper for tests: create a basic session with provided items/results.
    pub fn create_test_session_with_items(
        &mut self,
        now: u64,
        settings_generation: u64,
        session_id: &str,
        results: ScanResultsDto,
        items: HashMap<String, StoredItem>,
    ) {
        self.sessions.insert(
            session_id.to_string(),
            ScanSession {
                id: session_id.to_string(),
                created_at: now,
                settings_generation,
                results,
                items,
            },
        );
    }

    /// Create and store a scan session by running a read-only scan.
    pub fn start_scan<C: Clock>(
        &mut self,
        clock: &C,
        tiers: Option<Vec<TierDto>>,
        settings: &SettingsDto,
        settings_generation: u64,
    ) -> Result<StartScanResponse, ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        let enabled_tiers = resolve_enabled_tiers(tiers.as_deref(), settings);
        let targets = sweeper_knowledge::embedded_targets()
            .into_iter()
            .filter(|t| tier_enabled(t.tier, &enabled_tiers))
            .collect::<Vec<_>>();

        let report = scanner::scan_targets(targets, settings.min_age_hours).map_err(|e| {
            ContractError::Internal {
                reason: sanitize_reason(&e.to_string()),
            }
        })?;

        let session_id = self.next_id("scan", now);
        let (results, items) = map_report_to_session(&session_id, report);
        let session = ScanSession {
            id: session_id.clone(),
            created_at: now,
            settings_generation,
            results,
            items,
        };
        self.sessions.insert(session_id.clone(), session);
        Ok(StartScanResponse { session_id })
    }

    /// Get scan results for a valid session.
    pub fn get_scan_results<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<ScanResultsDto, ContractError> {
        self.cleanup(clock);
        let session = self.get_valid_session(clock, session_id, current_settings_generation)?;
        Ok(session.results.clone())
    }

    /// Create a preview token for selected item IDs (read-only; no clean).
    pub fn preview_clean<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        item_ids: Vec<String>,
        confirmations: Option<PreviewConfirmationsDto>,
        current_settings_generation: u64,
    ) -> Result<PreviewCleanResponse, ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        let session = self
            .get_valid_session(clock, session_id, current_settings_generation)?
            .clone();

        if item_ids.is_empty() {
            return Err(ContractError::InvalidInput {
                reason: "no items selected".to_string(),
            });
        }

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

        // Typed confirmation required for custom path / unknown selections.
        let needs_confirmation = selected.iter().any(|i| i.requires_typed_confirmation);
        if needs_confirmation {
            let ok = confirmations
                .and_then(|c| c.typed_confirmation)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !ok {
                return Err(ContractError::InvalidInput {
                    reason: "typed confirmation required".to_string(),
                });
            }
        }

        // Build breakdown by target.
        let mut by_target: BTreeMap<String, (String, u64, u64)> = BTreeMap::new();
        let mut total_count = 0u64;
        let mut total_bytes = 0u64;
        for item in &selected {
            let entry = by_target
                .entry(item.target_id.clone())
                .or_insert_with(|| (item.target_name.clone(), 0u64, 0u64));
            entry.1 = entry.1.saturating_add(1);
            entry.2 = entry.2.saturating_add(item.dto.size.get());
            total_count += 1;
            total_bytes = total_bytes.saturating_add(item.dto.size.get());
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

        let selection_hash = hash_selection(session_id, &item_ids);
        let token = self.next_id("preview", now);
        let state = PreviewTokenState {
            token: token.clone(),
            session_id: session_id.to_string(),
            issued_at: now,
            selection_hash,
            item_ids: item_ids.clone(),
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

    /// Validate (and consume) a preview token. `execute_clean` does not exist in Phase 2,
    /// but this helper supports Gate F expectations/tests.
    pub fn consume_preview_token<C: Clock>(
        &mut self,
        clock: &C,
        preview_token: &str,
        current_settings_generation: u64,
    ) -> Result<(String, Vec<String>), ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        let (session_id, item_ids, selection_hash, issued_at, used) = {
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
            )
        };
        if used {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token already used".to_string(),
            });
        }
        // Ensure session valid and bounds token.
        let _session = self.get_valid_session(clock, &session_id, current_settings_generation)?;
        if now.saturating_sub(issued_at) > PREVIEW_TOKEN_TTL_SECS {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token expired".to_string(),
            });
        }
        let expected = hash_selection(&session_id, &item_ids);
        if expected != selection_hash {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token tampered".to_string(),
            });
        }
        if let Some(t) = self.preview_tokens.get_mut(preview_token) {
            t.used = true;
        }
        Ok((session_id, item_ids))
    }

    /// Explain an item by id (scan item id or target/knowledge id).
    pub fn explain_item<C: Clock>(
        &mut self,
        clock: &C,
        item_id: &str,
        current_settings_generation: u64,
    ) -> Result<sweeper_contract::ExplainItemResponse, ContractError> {
        self.cleanup(clock);
        // Try as scan item in any valid session.
        let now = clock.now_unix_secs();
        for s in self.sessions.values() {
            if !session_valid_at(s, now, current_settings_generation) {
                continue;
            }
            if let Some(item) = s.items.get(item_id) {
                return Ok(sweeper_contract::ExplainItemResponse {
                    item_id: item_id.to_string(),
                    title: item.target_name.clone(),
                    description: lookup_target_description(&item.target_id)
                        .unwrap_or_else(|| "Scanned item.".to_string()),
                    risk: risk_to_dto(item.risk),
                });
            }
            // Also allow target-level explanation by target id present in session.
            if s.results.targets.iter().any(|t| t.target_id == item_id) {
                if let Some(t) = s.results.targets.iter().find(|t| t.target_id == item_id) {
                    return Ok(sweeper_contract::ExplainItemResponse {
                        item_id: item_id.to_string(),
                        title: t.target_name.clone(),
                        description: lookup_target_description(item_id)
                            .unwrap_or_else(|| "Target.".to_string()),
                        risk: t.risk,
                    });
                }
            }
        }
        // Fall back to knowledge base entries (always available).
        if let Some(k) = sweeper_knowledge::builtin_knowledge_entries()
            .into_iter()
            .find(|k| k.id == item_id)
        {
            return Ok(sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title: k.title,
                description: format!("{} {}", k.description, k.notes).trim().to_string(),
                risk: risk_to_dto(k.risk),
            });
        }
        // Fall back to embedded targets.
        if let Some(t) = sweeper_knowledge::embedded_targets()
            .into_iter()
            .find(|t| t.id == item_id)
        {
            return Ok(sweeper_contract::ExplainItemResponse {
                item_id: item_id.to_string(),
                title: t.name,
                description: t.description,
                risk: risk_to_dto(t.risk),
            });
        }
        Err(ContractError::NotFound {
            reason: "item not found".to_string(),
        })
    }

    fn get_valid_session<C: Clock>(
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
        if !session_valid_at(session, now, current_settings_generation) {
            return Err(ContractError::SessionInvalid {
                reason: "session expired or invalidated".to_string(),
            });
        }
        Ok(session)
    }
}

fn session_valid_at(session: &ScanSession, now: u64, current_settings_generation: u64) -> bool {
    if session.settings_generation != current_settings_generation {
        return false;
    }
    now.saturating_sub(session.created_at) <= SCAN_SESSION_TTL_SECS
}

fn resolve_enabled_tiers(
    requested: Option<&[TierDto]>,
    settings: &SettingsDto,
) -> HashSet<TierDto> {
    let mut set = HashSet::new();
    let base = |tier: TierDto, settings: &SettingsDto| match tier {
        TierDto::Tier1 => settings.enable_tier1,
        TierDto::Tier2 => settings.enable_tier2,
        TierDto::Tier3 => settings.enable_tier3,
    };
    match requested {
        Some(req) if !req.is_empty() => {
            for t in req {
                if base(*t, settings) {
                    set.insert(*t);
                }
            }
        }
        _ => {
            if settings.enable_tier1 {
                set.insert(TierDto::Tier1);
            }
            if settings.enable_tier2 {
                set.insert(TierDto::Tier2);
            }
            if settings.enable_tier3 {
                set.insert(TierDto::Tier3);
            }
        }
    }
    set
}

fn tier_enabled(tier: Tier, enabled: &HashSet<TierDto>) -> bool {
    match tier {
        Tier::Tier1 => enabled.contains(&TierDto::Tier1),
        Tier::Tier2 => enabled.contains(&TierDto::Tier2),
        Tier::Tier3 => enabled.contains(&TierDto::Tier3),
    }
}

fn tier_to_dto(t: Tier) -> TierDto {
    match t {
        Tier::Tier1 => TierDto::Tier1,
        Tier::Tier2 => TierDto::Tier2,
        Tier::Tier3 => TierDto::Tier3,
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
) -> (ScanResultsDto, HashMap<String, StoredItem>) {
    let mut items_map = HashMap::new();
    let targets = report
        .targets
        .into_iter()
        .map(|t| {
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
                            requires_typed_confirmation: false, // allowlist scan items only in Phase 2
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
    (results, items_map)
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

fn lookup_target_description(target_id: &str) -> Option<String> {
    sweeper_knowledge::embedded_targets()
        .into_iter()
        .find(|t| t.id == target_id)
        .map(|t| t.description)
}

fn sanitize_reason(s: &str) -> String {
    // Avoid leaking internal paths or stack traces in contract errors.
    // Keep it conservative and short.
    let lower = s.to_lowercase();
    if lower.contains("path")
        || lower.contains("\\")
        || lower.contains('/')
        || lower.contains("stack")
    {
        "operation failed".to_string()
    } else {
        // truncate
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
