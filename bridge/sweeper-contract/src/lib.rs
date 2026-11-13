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
    /// Minimum age in hours (default 24).
    pub min_age_hours: u32,
    /// Allow deleting unknown folders (advanced; still gated).
    pub allow_delete_unknown: bool,
}

impl Default for SettingsDto {
    fn default() -> Self {
        Self {
            enable_tier1: true,
            enable_tier2: true,
            enable_tier3: false,
            min_age_hours: 24,
            allow_delete_unknown: false,
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

    // Register/resolve all types that appear in command signatures and events.
    let _ = <StartScanResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ScanResultsDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <PreviewCleanResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ExplainItemResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <SettingsDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <LogMetadataDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <LogReadResponse as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ContractError as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ScanProgressEvent as NamedType>::definition_named_data_type(&mut type_map);
    // Types referenced transitively (enums, nested DTOs) should be pulled in, but ensure key ones.
    let _ = <TierDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <RiskDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <SkipReasonDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <ScanItemDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <TargetScanResultDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <PreviewConfirmationsDto as NamedType>::definition_named_data_type(&mut type_map);
    let _ = <PreviewTargetBreakdownDto as NamedType>::definition_named_data_type(&mut type_map);

    // Export named types (sorted by name for determinism as specta-typescript does internally? export() iterates map).
    let dependant_types = ts_cfg.export(&type_map)?;
    // ts_cfg.export includes header/framework_header; we need just the type defs section in tauri-specta layout.
    // But easier to strip headers and reconstruct full file in tauri-specta format.
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
    // specta-typescript default export prepends header + framework header + blank lines.
    // We keep everything after the initial comment/header block that belongs to types.
    // Simpler: remove known default framework header line if present.
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
    // Default ErrorHandlingMode in tauri-specta rc.21 is Result.
    // All commands return Result<T, ContractError> in Rust -> Promise<Result<T, ContractError>> in TS
    // with try/catch wrapper returning {status:"ok",data:...} or {status:"error",error:e as any}.
    let commands = [
        cmd_start_scan(cfg, type_map)?,
        cmd_get_scan_results(cfg, type_map)?,
        cmd_preview_clean(cfg, type_map)?,
        cmd_explain_item(cfg, type_map)?,
        cmd_get_settings(cfg, type_map)?,
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
    // tiers: Option<Vec<TierDto>>
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
    // ScanProgressEvent registered as event "ScanProgressEvent" by tauri-specta::Event derive macro name by default.
    let event_ty = datatype_str(
        cfg,
        <ScanProgressEvent as Type>::inline(type_map, Generics::NONE),
        type_map,
    )?;
    let event_name = "ScanProgressEvent";
    let event_camel = event_name.to_lower_camel_case();
    let events_types = format!("{event_camel}: {event_ty}");
    let events_map = format!("{event_camel}: \"{event_name}\"");
    Ok(format!(
        "export const events = __makeEvents__<{{\n{events_types}\n}}>({{\n{events_map}\n}})"
    ))
}

/// Deterministic serde fuzz entrypoint for contract payload bytes.
/// Accepts arbitrary bytes and must never panic.
pub fn fuzz_contract_payload_bytes(data: &[u8]) {
    // Try as UTF-8 JSON fragments for various DTOs/enums; errors are expected.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = serde_json::from_str::<U64String>(s);
        let _ = serde_json::from_str::<TierDto>(s);
        let _ = serde_json::from_str::<RiskDto>(s);
        let _ = serde_json::from_str::<SkipReasonDto>(s);
        let _ = serde_json::from_str::<ScanItemDto>(s);
        let _ = serde_json::from_str::<TargetScanResultDto>(s);
        let _ = serde_json::from_str::<ScanResultsDto>(s);
        let _ = serde_json::from_str::<StartScanResponse>(s);
        let _ = serde_json::from_str::<PreviewConfirmationsDto>(s);
        let _ = serde_json::from_str::<PreviewTargetBreakdownDto>(s);
        let _ = serde_json::from_str::<PreviewCleanResponse>(s);
        let _ = serde_json::from_str::<ExplainItemResponse>(s);
        let _ = serde_json::from_str::<SettingsDto>(s);
        let _ = serde_json::from_str::<LogMetadataDto>(s);
        let _ = serde_json::from_str::<LogReadResponse>(s);
        let _ = serde_json::from_str::<ScanProgressEvent>(s);
        let _ = serde_json::from_str::<ContractError>(s);
        let _ = serde_json::from_str::<Vec<String>>(s);
        let _ = serde_json::from_str::<Vec<TierDto>>(s);
    }
    // Also exercise raw-byte parsing attempts via serde_json::from_slice where meaningful.
    let _ = serde_json::from_slice::<U64String>(data);
    let _ = serde_json::from_slice::<ScanResultsDto>(data);
    let _ = serde_json::from_slice::<PreviewCleanResponse>(data);
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
