#![allow(missing_docs)] // docs preserved on items; module-level handled by lib

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use specta::datatype::DataType;
use specta::{Generics, Type, TypeCollection};

/// Decimal-string wrapper for `u64` values.
/// Serializes/deserializes as a JSON string to avoid JS number precision loss and BigInt issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct U64String(pub u64);

impl U64String {
    /// Returns the inner `u64`.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for U64String {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<U64String> for u64 {
    fn from(value: U64String) -> Self {
        value.0
    }
}

impl fmt::Display for U64String {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for U64String {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for U64String {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let v = u64::from_str(&s).map_err(serde::de::Error::custom)?;
        Ok(Self(v))
    }
}

impl Type for U64String {
    fn inline(type_map: &mut TypeCollection, generics: Generics<'_>) -> DataType {
        <String as Type>::inline(type_map, generics)
    }
}

/// Target tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum TierDto {
    /// System temp and caches.
    Tier1,
    /// Developer caches.
    Tier2,
    /// Heavy or risky items.
    Tier3,
}

/// Risk classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum RiskDto {
    /// Safe to remove when eligible.
    Safe,
    /// Recoverable but potentially costly.
    Caution,
    /// Not recognized.
    Unknown,
    /// Protected; never selectable.
    Protected,
}

/// Reason an item was skipped (scan/preview/clean contexts as applicable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum SkipReasonDto {
    /// Symlink or junction.
    LinkPoint,
    /// Cloud placeholder.
    CloudPlaceholder,
    /// Other reparse tag.
    OtherReparse,
    /// Unreadable.
    Unreadable,
    /// Too recent by minimum age.
    TooRecent,
    /// Session/token/validation issue or not eligible.
    NotEligible,
}

/// Aggregated skipped-reason entry for clean summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedReasonDto {
    /// Human-safe reason label/text.
    pub reason: String,
    /// Number of items skipped for this reason (decimal string).
    pub count: U64String,
}

/// Progress event for analyzer operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerProgressEvent {
    /// Analyzer session identifier.
    pub session_id: String,
    /// Number of directories scanned/enumerated (decimal string).
    pub scanned_dirs: U64String,
    /// Number of files observed (decimal string).
    pub scanned_files: U64String,
    /// Sum of logical file sizes observed in bytes (decimal string).
    pub scanned_bytes: U64String,
    /// Current/last path being processed (best-effort).
    pub current_path: Option<String>,
}

/// Analyzer tree node DTO (recursive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzerNodeDto {
    /// Stable node identifier (deterministic for canonical path).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Canonical/display path.
    pub path: String,
    /// Whether this node represents a directory/container in the tree.
    pub is_dir: bool,
    /// Recursive total logical size in bytes (decimal string).
    pub size_bytes: U64String,
    /// Recursive item count under/including this node as applicable (decimal string).
    pub item_count: U64String,
    /// Risk classification.
    pub risk: RiskDto,
    /// Knowledge-base/annotation description.
    pub description: String,
    /// Selection/annotation reason (diagnostic, if any).
    pub reason: Option<String>,
    /// Owning tier when known.
    pub tier: Option<TierDto>,
    /// Matched allowlist target id when known.
    pub target_id: Option<String>,
    /// True if the node itself is a `Link Point`.
    pub is_link_point: bool,
    /// True if the node itself is a cloud placeholder.
    pub is_cloud_placeholder: bool,
    /// True if display-only (never cleanable in v1 flows).
    pub display_only: bool,
    /// Whether selectable for routing into standard Preview -> Clean.
    pub selectable: bool,
    /// Child nodes.
    pub children: Vec<AnalyzerNodeDto>,
}

/// Response from starting an analyzer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartAnalyzerResponse {
    /// Analyzer session identifier.
    pub session_id: String,
    /// Canonical/root path analyzed.
    pub root_path: String,
    /// Total recursive bytes for the root (decimal string).
    pub total_bytes: U64String,
    /// Total recursive items for the root (decimal string).
    pub total_items: U64String,
    /// Directories scanned (decimal string).
    pub scanned_dirs: U64String,
    /// Files scanned (decimal string).
    pub scanned_files: U64String,
    /// Whether the analysis was cancelled before completion.
    pub cancelled: bool,
    /// Non-fatal warnings.
    pub warnings: Vec<String>,
    /// Root tree node.
    pub root: AnalyzerNodeDto,
}

/// Verdict from custom path validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ValidateCustomPathResponse {
    /// Normalized/canonical path when available, otherwise the input.
    pub path: String,
    /// Whether the path is allowed as a custom path root.
    pub allowed: bool,
    /// Human-safe reason/verdict text.
    pub reason: String,
}

/// Display metadata for a scanned item (path is display-only; never submitted for deletion).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanItemDto {
    /// Stable per-scan item identifier.
    pub id: String,
    /// Owning target identifier.
    pub target_id: String,
    /// Display path.
    pub path: String,
    /// Logical size in bytes (decimal string).
    pub size: U64String,
    /// Last modified time as unix seconds (decimal string).
    pub mtime_unix_secs: U64String,
    /// Whether eligible by minimum age.
    pub age_eligible: bool,
}

/// Per-target scan result for UI display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TargetScanResultDto {
    /// Target identifier.
    pub target_id: String,
    /// Human-readable target name.
    pub target_name: String,
    /// Tier.
    pub tier: TierDto,
    /// Risk.
    pub risk: RiskDto,
    /// Resolved concrete paths scanned (display).
    pub resolved_paths: Vec<String>,
    /// Whether scanned.
    pub scanned: bool,
    /// Status/disable reason.
    pub status_reason: Option<String>,
    /// Total bytes (decimal string).
    pub total_bytes: U64String,
    /// Eligible bytes (decimal string).
    pub eligible_bytes: U64String,
    /// Total file count (decimal string).
    pub file_count: U64String,
    /// Eligible file count (decimal string).
    pub eligible_count: U64String,
    /// Skipped count (decimal string).
    pub skipped_count: U64String,
    /// Items for this target.
    pub items: Vec<ScanItemDto>,
}

/// Scan results payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScanResultsDto {
    /// Session identifier.
    pub session_id: String,
    /// When scan started (unix seconds, decimal string).
    pub started_unix_secs: U64String,
    /// Minimum age applied (hours).
    pub min_age_hours: u32,
    /// Per-target results.
    pub targets: Vec<TargetScanResultDto>,
}

/// Response from starting a scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StartScanResponse {
    /// Created scan session identifier.
    pub session_id: String,
}

/// Confirmation payload for preview when required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreviewConfirmationsDto {
    /// Typed confirmation for custom path / unknown selections when required.
    pub typed_confirmation: Option<String>,
}

/// Per-target breakdown entry in preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTargetBreakdownDto {
    /// Target identifier.
    pub target_id: String,
    /// Target name.
    pub target_name: String,
    /// Selected item count (decimal string).
    pub selected_count: U64String,
    /// Selected total bytes estimate (decimal string).
    pub selected_bytes: U64String,
}

/// Preview response including single-use preview token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCleanResponse {
    /// Preview token (single-use, TTL-bounded).
    pub preview_token: String,
    /// Per-target breakdown.
    pub breakdown: Vec<PreviewTargetBreakdownDto>,
    /// Total selected items (decimal string).
    pub total_selected_count: U64String,
    /// Total selected bytes estimate (decimal string).
    pub total_selected_bytes: U64String,
    /// Number of selected items skipped during preview validation (decimal string).
    #[serde(default)]
    pub skipped_count: U64String,
    /// Distinct skip reasons recorded during preview validation.
    #[serde(default)]
    pub skipped_reasons: Vec<String>,
}

/// Progress event for clean operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanProgressEvent {
    /// Preview token for the active clean.
    pub preview_token: String,
    /// Target identifier for the current/last processed item.
    pub target_id: String,
    /// Completed item count (decimal string).
    pub completed_items: U64String,
    /// Total item count (decimal string).
    pub total_items: U64String,
    /// Bytes freed so far (decimal string).
    pub bytes_freed_so_far: U64String,
    /// Status label for the current progress update.
    pub status: String,
}

/// Final response from executing a clean operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteCleanResponse {
    /// Bytes freed (decimal string).
    pub bytes_freed: U64String,
    /// Deleted item count (decimal string).
    pub deleted_count: U64String,
    /// Skipped item count (decimal string).
    pub skipped_count: U64String,
    /// Aggregated skipped reasons.
    pub skipped_reasons: Vec<SkippedReasonDto>,
    /// Whether clean aborted early.
    pub aborted: bool,
    /// Abort reason if aborted.
    pub abort_reason: Option<String>,
    /// Associated log identifier for viewing operation log.
    pub log_id: Option<String>,
}

/// Explanation for an item or target (read-only knowledge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExplainItemResponse {
    /// Subject identifier.
    pub item_id: String,
    /// Title/name.
    pub title: String,
    /// Description.
    pub description: String,
    /// Risk classification.
    pub risk: RiskDto,
}

/// Elevation status for the current process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ElevationStatusDto {
    /// Whether the current process is running elevated (administrator).
    pub is_elevated: bool,
}

/// Result of attempting to relaunch the app elevated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelaunchResultDto {
    /// Whether a relaunch was initiated/launched.
    pub launched: bool,
}

/// Settings DTO (validated, safe defaults on invalid input at bridge layer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    /// Whether Tier 1 targets are enabled.
    pub enable_tier1: bool,
    /// Whether Tier 2 targets are enabled.
    pub enable_tier2: bool,
    /// Whether Tier 3 targets are enabled (disabled by default in v1).
    pub enable_tier3: bool,
    /// Allow elevation-gated targets (requires running elevated process). Default false.
    #[serde(default)]
    pub enable_elevation_targets: bool,
    /// Minimum age in hours (default 24). Values below 24 are allowed but should be warned in UI.
    pub min_age_hours: u32,
    /// Allow deleting/cleaning `Unknown` analyzer selections when explicitly enabled and confirmed.
    pub allow_delete_unknown: bool,
    /// Validated canonical Custom Path roots (contents-only). Protected paths are never stored.
    pub custom_paths: Vec<String>,
    /// Project staleness threshold in days for Tier 3 stale Rust `target/` and `node_modules` discovery.
    pub stale_project_days: u32,
}

impl Default for SettingsDto {
    fn default() -> Self {
        Self {
            enable_tier1: true,
            enable_tier2: true,
            enable_tier3: false,
            enable_elevation_targets: false,
            min_age_hours: 24,
            allow_delete_unknown: false,
            custom_paths: Vec::new(),
            stale_project_days: 30,
        }
    }
}

/// Log file metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogMetadataDto {
    /// Log identifier (filename stem or stable id).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Last modified unix seconds (decimal string) if known.
    pub modified_unix_secs: Option<U64String>,
    /// Size in bytes (decimal string) if known.
    pub size_bytes: Option<U64String>,
}

/// Log contents response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogReadResponse {
    /// Log identifier.
    pub id: String,
    /// Raw log text (JSON lines or plain text as stored).
    pub content: String,
}

/// Progress event for scan operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ScanProgressEvent {
    /// Scan started.
    Started {
        /// Session identifier.
        session_id: String,
    },
    /// Incremental scan progress.
    Progress {
        /// Session identifier.
        session_id: String,
        /// Number of targets completed so far.
        completed_targets: u32,
        /// Total number of targets in this scan run.
        total_targets: u32,
        /// Currently scanning target id, if any.
        current_target_id: Option<String>,
        /// Currently scanning target name, if any.
        current_target_name: Option<String>,
        /// Aggregate scanned/observed items so far (decimal string).
        scanned_items: String,
        /// Aggregate scanned/observed bytes so far (decimal string).
        scanned_bytes: String,
    },
    /// Scan cancelled.
    Cancelled {
        /// Session identifier.
        session_id: String,
    },
    /// Scan completed.
    Completed {
        /// Session identifier.
        session_id: String,
    },
    /// Scan failed.
    Failed {
        /// Session identifier.
        session_id: String,
        /// Structured error.
        error: ContractError,
    },
}

/// Structured contract error variants (no internal paths or stack traces).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", content = "details", rename_all = "camelCase")]
pub enum ContractError {
    /// Session not found or expired/invalidated.
    #[error("session invalid")]
    SessionInvalid {
        /// Human-safe reason.
        reason: String,
    },
    /// One or more item IDs were unknown for the session.
    #[error("unknown item")]
    UnknownItem {
        /// Count of unknown items (decimal string).
        count: U64String,
    },
    /// Preview token invalid, expired, tampered, or already used.
    #[error("preview token invalid")]
    PreviewTokenInvalid {
        /// Human-safe reason.
        reason: String,
    },
    /// Input validation failed.
    #[error("invalid input")]
    InvalidInput {
        /// Human-safe reason.
        reason: String,
    },
    /// Requested resource not found.
    #[error("not found")]
    NotFound {
        /// Human-safe reason.
        reason: String,
    },
    /// Operation failed internally (sanitized).
    #[error("internal error")]
    Internal {
        /// Human-safe reason.
        reason: String,
    },
}
