#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Contract DTOs, error types, command signatures, and TypeScript export for the Sweeper bridge.
//!
//! This crate is the single source of truth for IPC shapes shared by `sweeper-desktop` and
//! `frontend`. It has no dependency on `sweeper-winfs`, `sweeper-core`, or `sweeper-knowledge`.

use std::fmt;
use std::str::FromStr;

use heck::ToLowerCamelCase;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use specta::datatype::{DataType, FunctionResultVariant};
use specta::{Generics, NamedType, Type, TypeCollection};

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
    fn inline(type_map: &mut TypeCollection, generics: Generics<'_>) -> specta::datatype::DataType {
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

/// Deterministic TypeScript export for the contract bindings.
///
/// Writes bindings to `frontend/src/contract/bindings.ts` relative to the workspace/repo root.
pub fn export_ts_bindings() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("frontend")
        .join("src")
        .join("contract")
        .join("bindings.ts");
    export_ts_bindings_to(&path)
}

/// Export TypeScript bindings to a specific path (for tests).
pub fn export_ts_bindings_to(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = render_bindings()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

fn render_bindings() -> Result<String, Box<dyn std::error::Error>> {
    use specta_typescript as ts;

    let ts_cfg = ts::Typescript::default();
    let mut type_map = TypeCollection::default();

    let _ = <StartScanResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ScanResultsDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <PreviewCleanResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ExplainItemResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <SettingsDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ElevationStatusDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <RelaunchResultDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <LogMetadataDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <LogReadResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ContractError as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ScanProgressEvent as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ExecuteCleanResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <CleanProgressEvent as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <AnalyzerProgressEvent as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <AnalyzerNodeDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <StartAnalyzerResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ValidateCustomPathResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <TierDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <RiskDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <SkipReasonDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ScanItemDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <TargetScanResultDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <PreviewConfirmationsDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <PreviewTargetBreakdownDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <SkippedReasonDto as NamedType>::definition_named_data_type(&mut type_map);

    let dependant_types = ts_cfg.export(&type_map)?;
    let types_only = strip_specta_typescript_headers(&dependant_types)
        .trim()
        .to_string();

    let commands = render_commands(&ts_cfg, &mut type_map)?;
    let events = render_events(&ts_cfg, &mut type_map)?;

    let globals = include_str!("bindings_globals.ts").trim_start_matches('\u{feff}');

    const DO_NOT_EDIT: &str = "// This file was generated by [tauri-specta](https://github.com/oscartbeaumont/tauri-specta). Do not edit this file manually.";

    let content = format!(
        "\n{DO_NOT_EDIT}\n\n/** user-defined commands **/\n\n{commands}\n\n/** user-defined events **/\n\n{events}\n\n/** user-defined constants **/\n\n\n\n/** user-defined types **/\n\n{types_only}\n\n/** tauri-specta globals **/\n\n{globals}"
    );
    Ok(content.replace("\r\n", "\n"))
}

fn strip_specta_typescript_headers(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(stripped) =
        out.strip_prefix("// This file has been generated by Specta. DO NOT EDIT.\n\n")
    {
        out = stripped.to_string();
    } else if let Some(stripped) =
        out.strip_prefix("// This file has been generated by Specta. DO NOT EDIT.\n")
    {
        out = stripped.trim_start_matches('\n').to_string();
    }
    out
}

fn datatype_str(
    cfg: &specta_typescript::Typescript,
    dt: DataType,
    type_map: &TypeCollection,
) -> Result<String, specta_typescript::ExportError> {
    specta_typescript::datatype(cfg, &FunctionResultVariant::Value(dt), type_map)
}

fn render_commands(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let commands = [
        cmd_start_scan(cfg, type_map)?,
        cmd_get_scan_results(cfg, type_map)?,
        cmd_preview_clean(cfg, type_map)?,
        cmd_execute_clean(cfg, type_map)?,
        cmd_start_analyzer(cfg, type_map)?,
        cmd_explain_item(cfg, type_map)?,
        cmd_get_settings(cfg, type_map)?,
        cmd_update_settings(cfg, type_map)?,
        cmd_validate_custom_path(cfg, type_map)?,
        cmd_list_logs(cfg, type_map)?,
        cmd_read_log(cfg, type_map)?,
    ];
    Ok(format!(
        "export const commands = {{\n{}\n}}",
        commands.join(",\n")
    ))
}

fn cmd_fn(
    name: &str,
    args: &[(&str, String)],
    ret_promise_inner: &str,
    invoke_name: &str,
    arg_obj: Option<&str>,
) -> String {
    let args_str = args
        .iter()
        .map(|(n, t)| format!("{}: {}", n.to_lower_camel_case(), t))
        .collect::<Vec<_>>()
        .join(", ");
    let arg_usage = arg_obj.map(|s| format!(", {s}")).unwrap_or_default();
    format!(
        "async {name}({args_str}) : Promise<{ret_promise_inner}> {{\n    try {{\n    return {{ status: \"ok\", data: await TAURI_INVOKE(\"{invoke_name}\"{arg_usage}) }};\n}} catch (e) {{\n    if(e instanceof Error) throw e;\n    else return {{ status: \"error\", error: e  as any }};\n}}\n}}"
    )
}

fn cmd_start_scan(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let tiers_ty = datatype_str(
        cfg,
        <Option<Vec<TierDto>> as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let ok_ty = datatype_str(
        cfg,
        <StartScanResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "startScan",
        &[("tiers", tiers_ty)],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "start_scan",
        Some("{ tiers }"),
    ))
}

fn cmd_get_scan_results(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let ok_ty = datatype_str(
        cfg,
        <ScanResultsDto as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "getScanResults",
        &[("session_id", "string".to_string())],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "get_scan_results",
        Some("{ sessionId }"),
    ))
}

fn cmd_preview_clean(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let item_ids_ty = datatype_str(
        cfg,
        <Vec<String> as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let confirmations_ty = datatype_str(
        cfg,
        <Option<PreviewConfirmationsDto> as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let ok_ty = datatype_str(
        cfg,
        <PreviewCleanResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "previewClean",
        &[
            ("session_id", "string".to_string()),
            ("item_ids", item_ids_ty),
            ("confirmations", confirmations_ty),
        ],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "preview_clean",
        Some("{ sessionId, itemIds, confirmations }"),
    ))
}

fn cmd_execute_clean(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let ok_ty = datatype_str(
        cfg,
        <ExecuteCleanResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "executeClean",
        &[("preview_token", "string".to_string())],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "execute_clean",
        Some("{ previewToken }"),
    ))
}

fn cmd_start_analyzer(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let root_ty = datatype_str(
        cfg,
        <Option<String> as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let ok_ty = datatype_str(
        cfg,
        <StartAnalyzerResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "startAnalyzer",
        &[("root", root_ty)],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "start_analyzer",
        Some("{ root }"),
    ))
}

fn cmd_explain_item(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let ok_ty = datatype_str(
        cfg,
        <ExplainItemResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "explainItem",
        &[("item_id", "string".to_string())],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "explain_item",
        Some("{ itemId }"),
    ))
}

fn cmd_get_settings(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let ok_ty = datatype_str(
        cfg,
        <SettingsDto as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "getSettings",
        &[],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "get_settings",
        None,
    ))
}

fn cmd_update_settings(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let settings_ty = datatype_str(
        cfg,
        <SettingsDto as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let ok_ty = datatype_str(
        cfg,
        <SettingsDto as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "updateSettings",
        &[("settings", settings_ty)],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "update_settings",
        Some("{ settings }"),
    ))
}

fn cmd_validate_custom_path(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let ok_ty = datatype_str(
        cfg,
        <ValidateCustomPathResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "validateCustomPath",
        &[("path", "string".to_string())],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "validate_custom_path",
        Some("{ path }"),
    ))
}

fn cmd_list_logs(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let list_ty = datatype_str(
        cfg,
        <Vec<LogMetadataDto> as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "listLogs",
        &[],
        &format!("Result<{list_ty}, {err_ty}>"),
        "list_logs",
        None,
    ))
}

fn cmd_read_log(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let ok_ty = datatype_str(
        cfg,
        <LogReadResponse as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let err_ty = datatype_str(
        cfg,
        <ContractError as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    Ok(cmd_fn(
        "readLog",
        &[("log_id", "string".to_string())],
        &format!("Result<{ok_ty}, {err_ty}>"),
        "read_log",
        Some("{ logId }"),
    ))
}

fn render_events(
    cfg: &specta_typescript::Typescript,
    type_map: &mut TypeCollection,
) -> Result<String, Box<dyn std::error::Error>> {
    let scan_ty = datatype_str(
        cfg,
        <ScanProgressEvent as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let clean_ty = datatype_str(
        cfg,
        <CleanProgressEvent as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let analyzer_ty = datatype_str(
        cfg,
        <AnalyzerProgressEvent as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let events_types = format!(
        "scanProgressEvent: {scan_ty},\ncleanProgressEvent: {clean_ty},\nanalyzerProgressEvent: {analyzer_ty}"
    );
    let events_map =
        "scanProgressEvent: \"ScanProgressEvent\",\ncleanProgressEvent: \"CleanProgressEvent\",\nanalyzerProgressEvent: \"AnalyzerProgressEvent\""
            .to_string();
    Ok(format!(
        "export const events = __makeEvents__<{{\n{events_types}\n}}>({{\n{events_map}\n}})"
    ))
}

/// Deterministic serde fuzz entrypoint for contract payload bytes.
/// Accepts arbitrary bytes and must never panic.
pub fn fuzz_contract_payload_bytes(data: &[u8]) {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<U64String>(s);
        let _ = serde_json::from_str::<TierDto>(s);
        let _ = serde_json::from_str::<RiskDto>(s);
        let _ = serde_json::from_str::<SkipReasonDto>(s);
        let _ = serde_json::from_str::<SkippedReasonDto>(s);
        let _ = serde_json::from_str::<ScanItemDto>(s);
        let _ = serde_json::from_str::<TargetScanResultDto>(s);
        let _ = serde_json::from_str::<ScanResultsDto>(s);
        let _ = serde_json::from_str::<StartScanResponse>(s);
        let _ = serde_json::from_str::<AnalyzerNodeDto>(s);
        let _ = serde_json::from_str::<AnalyzerProgressEvent>(s);
        let _ = serde_json::from_str::<StartAnalyzerResponse>(s);
        let _ = serde_json::from_str::<ValidateCustomPathResponse>(s);
        let _ = serde_json::from_str::<PreviewConfirmationsDto>(s);
        let _ = serde_json::from_str::<PreviewTargetBreakdownDto>(s);
        let _ = serde_json::from_str::<PreviewCleanResponse>(s);
        let _ = serde_json::from_str::<ExecuteCleanResponse>(s);
        let _ = serde_json::from_str::<ExplainItemResponse>(s);
        let _ = serde_json::from_str::<SettingsDto>(s);
        let _ = serde_json::from_str::<LogMetadataDto>(s);
        let _ = serde_json::from_str::<LogReadResponse>(s);
        let _ = serde_json::from_str::<ScanProgressEvent>(s);
        let _ = serde_json::from_str::<CleanProgressEvent>(s);
        let _ = serde_json::from_str::<ContractError>(s);
        let _ = serde_json::from_str::<ElevationStatusDto>(s);
        let _ = serde_json::from_str::<RelaunchResultDto>(s);
        let _ = serde_json::from_str::<Vec<String>>(s);
        let _ = serde_json::from_str::<Vec<TierDto>>(s);
    }
    let _ = serde_json::from_slice::<U64String>(data);
    let _ = serde_json::from_slice::<ScanResultsDto>(data);
    let _ = serde_json::from_slice::<AnalyzerNodeDto>(data);
    let _ = serde_json::from_slice::<StartAnalyzerResponse>(data);
    let _ = serde_json::from_slice::<PreviewCleanResponse>(data);
    let _ = serde_json::from_slice::<ExecuteCleanResponse>(data);
    let _ = serde_json::from_slice::<CleanProgressEvent>(data);
    let _ = serde_json::from_slice::<ContractError>(data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn u64_string_serializes_as_decimal_string() {
        let v = U64String(9_007_199_254_740_993);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"9007199254740993\"");
        let de: U64String = serde_json::from_str(&json).unwrap();
        assert_eq!(de, v);
    }

    #[test]
    fn dto_u64_fields_roundtrip_as_strings() {
        let item = ScanItemDto {
            id: "t::1".to_string(),
            target_id: "user-temp".to_string(),
            path: r"C:\Users\X\AppData\Local\Temp\a.tmp".to_string(),
            size: U64String(123),
            mtime_unix_secs: U64String(456),
            age_eligible: true,
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(v["size"], serde_json::json!("123"));
        assert_eq!(v["mtimeUnixSecs"], serde_json::json!("456"));
        let back: ScanItemDto = serde_json::from_value(v).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn clean_dtos_serialize_u64_fields_as_strings() {
        let event = CleanProgressEvent {
            preview_token: "pt".to_string(),
            target_id: "t1".to_string(),
            completed_items: U64String(1),
            total_items: U64String(2),
            bytes_freed_so_far: U64String(3),
            status: "running".to_string(),
        };
        let v = serde_json::to_value(&event).unwrap();
        assert_eq!(v["completedItems"], serde_json::json!("1"));
        assert_eq!(v["totalItems"], serde_json::json!("2"));
        assert_eq!(v["bytesFreedSoFar"], serde_json::json!("3"));
        let resp = ExecuteCleanResponse {
            bytes_freed: U64String(10),
            deleted_count: U64String(2),
            skipped_count: U64String(1),
            skipped_reasons: vec![SkippedReasonDto {
                reason: "Skipped: locked".to_string(),
                count: U64String(1),
            }],
            aborted: false,
            abort_reason: None,
            log_id: Some("log-1".to_string()),
        };
        let vr = serde_json::to_value(&resp).unwrap();
        assert_eq!(vr["bytesFreed"], serde_json::json!("10"));
        assert_eq!(vr["deletedCount"], serde_json::json!("2"));
        assert_eq!(vr["skippedCount"], serde_json::json!("1"));
        assert_eq!(vr["skippedReasons"][0]["count"], serde_json::json!("1"));
    }

    #[test]
    fn analyzer_dtos_serialize_u64_fields_as_strings() {
        let event = AnalyzerProgressEvent {
            session_id: "a1".to_string(),
            scanned_dirs: U64String(2),
            scanned_files: U64String(3),
            scanned_bytes: U64String(4),
            current_path: Some(r"C:\Temp".to_string()),
        };
        let ev = serde_json::to_value(&event).unwrap();
        assert_eq!(ev["sessionId"], serde_json::json!("a1"));
        assert_eq!(ev["scannedDirs"], serde_json::json!("2"));
        assert_eq!(ev["scannedFiles"], serde_json::json!("3"));
        assert_eq!(ev["scannedBytes"], serde_json::json!("4"));

        let node = AnalyzerNodeDto {
            id: "n1".to_string(),
            name: "Temp".to_string(),
            path: r"C:\Temp".to_string(),
            is_dir: true,
            size_bytes: U64String(10),
            item_count: U64String(2),
            risk: RiskDto::Safe,
            description: "desc".to_string(),
            reason: None,
            tier: Some(TierDto::Tier1),
            target_id: Some("user-temp".to_string()),
            is_link_point: false,
            is_cloud_placeholder: false,
            display_only: false,
            selectable: true,
            children: vec![],
        };
        let resp = StartAnalyzerResponse {
            session_id: "a1".to_string(),
            root_path: r"C:\Temp".to_string(),
            total_bytes: U64String(10),
            total_items: U64String(2),
            scanned_dirs: U64String(1),
            scanned_files: U64String(1),
            cancelled: false,
            warnings: vec![],
            root: node.clone(),
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["totalBytes"], serde_json::json!("10"));
        assert_eq!(v["totalItems"], serde_json::json!("2"));
        assert_eq!(v["scannedDirs"], serde_json::json!("1"));
        assert_eq!(v["scannedFiles"], serde_json::json!("1"));
        assert_eq!(v["root"]["sizeBytes"], serde_json::json!("10"));
        assert_eq!(v["root"]["itemCount"], serde_json::json!("2"));
        let _roundtrip: StartAnalyzerResponse = serde_json::from_value(v).unwrap();
    }

    #[test]
    fn fuzz_entrypoint_never_panics_on_smoke_inputs() {
        for sample in [
            &b""[..],
            b"null",
            b"\"123\"",
            b"{\"bad\":true}",
            b"\xff\xfe\x00\x00",
        ] {
            fuzz_contract_payload_bytes(sample);
        }
    }

    #[test]
    fn export_ts_bindings_matches_committed_file() {
        let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("frontend")
            .join("src")
            .join("contract")
            .join("bindings.ts");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        export_ts_bindings_to(tmp.path()).expect("export bindings");
        let generated = fs::read_to_string(tmp.path()).unwrap();
        assert!(
            !generated.contains('\u{feff}'),
            "generated bindings must not contain UTF-8 BOM"
        );
        let existing = fs::read_to_string(&committed)
            .unwrap_or_else(|_| panic!("missing committed bindings at {}", committed.display()));
        assert!(
            !existing.contains('\u{feff}'),
            "committed bindings must not contain UTF-8 BOM"
        );
        let existing_normalized = existing.replace("\r\n", "\n");
        assert_eq!(
            generated, existing_normalized,
            "TypeScript bindings drift detected; re-export frontend/src/contract/bindings.ts"
        );
    }
}
