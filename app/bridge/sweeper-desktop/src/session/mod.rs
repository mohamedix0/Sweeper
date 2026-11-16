//! In-memory scan sessions and preview tokens with TTL enforcement.

pub(crate) mod analyzer;
pub(crate) mod clean;
mod mapping;
pub(crate) mod scan;
mod support;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sweeper_contract::{ScanItemDto, ScanResultsDto};
use sweeper_core::analyzer::AnalyzerResult;
use sweeper_core::remover::ValidatedItem;
use sweeper_core::settings::SweeperSettings;
use sweeper_core::{Risk, Tier};

use crate::clock::Clock;

pub(crate) use clean::PreviewCleanRequest;
pub(crate) use mapping::map_analyzer_error_pub;
pub(crate) use support::{sanitize_reason_pub, MsgRef};

pub(super) const SCAN_SESSION_TTL_SECS: u64 = 15 * 60;
pub(super) const PREVIEW_TOKEN_TTL_SECS: u64 = 10 * 60;
pub(super) const TYPED_CONFIRMATION_PHRASE: &str = "DELETE";
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

/// Cohesive input for inserting/storing an analyzer session.
#[derive(Debug, Clone)]
pub struct AnalyzerSessionInput {
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
    pub(super) sessions: HashMap<String, ScanSession>,
    pub(super) analyzer_sessions: HashMap<String, AnalyzerSession>,
    pub(super) preview_tokens: HashMap<String, PreviewTokenState>,
    pub(super) active_scans: HashMap<String, Arc<AtomicBool>>,
    pub(super) active_analyzers: HashMap<String, Arc<AtomicBool>>,
    pub(super) counter: u64,
}

/// Prepared scan registration (returned under short lock; scan runs without holding store).
#[derive(Debug, Clone)]
pub struct PreparedScan {
    /// Pre-allocated session id (used for events and stored session on success).
    pub session_id: String,
    /// Effective settings snapshot for this scan run.
    pub effective_settings: SweeperSettings,
    /// Settings generation snapshot at scan start.
    pub settings_generation: u64,
    /// Cooperative cancellation token for this scan.
    pub cancel: Arc<AtomicBool>,
}

/// Prepared analyzer registration (returned under short lock; analysis runs without holding store).
#[derive(Debug, Clone)]
pub struct PreparedAnalyzer {
    /// Pre-allocated session id.
    pub session_id: String,
    /// Settings generation snapshot at analyzer start.
    pub settings_generation: u64,
    /// Cooperative cancellation token.
    pub cancel: Arc<AtomicBool>,
}

/// Cohesive input for committing a completed scan session.
#[derive(Debug)]
pub struct ScanFinishInput {
    /// Session id.
    pub session_id: String,
    /// Settings generation snapshot at scan start.
    pub settings_generation_at_start: u64,
    /// Current settings generation at commit time.
    pub current_settings_generation: u64,
    /// Effective minimum age hours for this scan.
    pub effective_min_age_hours: u32,
    /// Completed scan report.
    pub report: sweeper_core::scanner::ScanReport,
}

/// Cohesive input for committing a completed analyzer session.
#[derive(Debug)]
pub struct AnalyzerFinishInput {
    /// Session id.
    pub session_id: String,
    /// Settings generation snapshot at analyzer start.
    pub settings_generation_at_start: u64,
    /// Current settings generation at commit time.
    pub current_settings_generation: u64,
    /// Minimum age hours at analysis time.
    pub min_age_hours: u32,
    /// Completed analyzer result.
    pub result: AnalyzerResult,
}

/// Cohesive request for issuing a preview token.
#[derive(Debug)]
pub struct PreviewTokenSpec {
    /// Token issue time (unix secs).
    pub now: u64,
    /// Session id the token binds to.
    pub session_id: String,
    /// Selected item IDs.
    pub item_ids: Vec<String>,
    /// Validated items for clean execution.
    pub validated_items: Vec<ValidatedItem>,
    /// Estimated bytes for the previewed selection.
    pub total_bytes: u64,
    /// Per-target breakdown for UI.
    pub breakdown: Vec<sweeper_contract::PreviewTargetBreakdownDto>,
}

/// Cohesive request for executing an already-validated clean batch.
#[derive(Debug)]
pub struct ValidatedCleanBatch {
    /// Consumed preview token id.
    pub preview_token: String,
    /// Minimum age hours for clean options.
    pub min_age_hours: u32,
    /// Validated items to remove.
    pub validated_items: Vec<ValidatedItem>,
    /// Preview estimated bytes.
    pub preview_estimated_bytes: u64,
    /// Optional journal/log base directory.
    pub journal_base_dir: Option<std::path::PathBuf>,
}

/// Test-only seed for creating a scan session with prepared items/results.
#[derive(Debug, Clone)]
pub struct TestScanSessionSeed {
    /// Creation time (unix secs).
    pub now: u64,
    /// Settings generation at creation.
    pub settings_generation: u64,
    /// Session id.
    pub session_id: String,
    /// Scan results DTO.
    pub results: ScanResultsDto,
    /// Items by id.
    pub items: HashMap<String, StoredItem>,
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
        for c in self.active_scans.values() {
            c.store(true, Ordering::Relaxed);
        }
        for c in self.active_analyzers.values() {
            c.store(true, Ordering::Relaxed);
        }
        self.active_scans.clear();
        self.active_analyzers.clear();
        self.sessions.clear();
        self.analyzer_sessions.clear();
        self.preview_tokens.clear();
    }

    /// Direct access helper for tests: create a basic session with provided items/results.
    pub fn create_test_session_with_items(&mut self, seed: TestScanSessionSeed) {
        let target_roots = seed
            .results
            .targets
            .iter()
            .map(|t| (t.target_id.clone(), t.resolved_paths.clone()))
            .collect();
        self.sessions.insert(
            seed.session_id.clone(),
            ScanSession {
                id: seed.session_id,
                created_at: seed.now,
                settings_generation: seed.settings_generation,
                results: seed.results,
                items: seed.items,
                target_roots,
                min_age_hours: 24,
            },
        );
    }

    /// Direct access helper for integration tests: insert a synthetic analyzer session.
    #[doc(hidden)]
    pub fn insert_test_analyzer_session(&mut self, input: AnalyzerSessionInput) {
        self.insert_analyzer_session(input);
    }

    /// Insert an analyzer session from cohesive input.
    pub fn insert_analyzer_session(&mut self, input: AnalyzerSessionInput) {
        let session = AnalyzerSession {
            id: input.id.clone(),
            created_at: input.created_at,
            settings_generation: input.settings_generation,
            result: input.result,
            min_age_hours: input.min_age_hours,
        };
        self.analyzer_sessions.insert(session.id.clone(), session);
    }
}
// modules declared at end of file after type definitions - removed duplicates here (will add once below)
