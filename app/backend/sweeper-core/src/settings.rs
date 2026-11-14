//! Settings validation and persistence helpers.
//!
//! Settings JSON is treated as untrusted input and re-validated/sanitized on every load.
//! `Protected Path`s are never stored in or overridable via settings.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths::CanonicalPath;

/// Maximum allowed `minimum_age_hours` (1 year).
pub const MAX_MIN_AGE_HOURS: u32 = 24 * 365;
/// Default `minimum_age_hours`.
pub const DEFAULT_MIN_AGE_HOURS: u32 = 24;
/// Default `stale_project_days` for Tier 3 project staleness discovery.
pub const DEFAULT_STALE_PROJECT_DAYS: u32 = 30;
/// Minimum allowed `stale_project_days`.
pub const MIN_STALE_PROJECT_DAYS: u32 = 1;
/// Maximum allowed `stale_project_days` (~10 years).
pub const MAX_STALE_PROJECT_DAYS: u32 = 3650;

/// Validated, safe-by-default settings model for scan/clean/analyzer behavior.
///
/// Fields are independently sanitized when loaded from untrusted JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SweeperSettings {
    /// Enable Tier 1 targets.
    pub enable_tier1: bool,
    /// Enable Tier 2 targets.
    pub enable_tier2: bool,
    /// Enable Tier 3 targets (strictly opt-in, disabled by default).
    pub enable_tier3: bool,
    /// Minimum file age in hours before eligibility.
    /// Bounded to `0..=8760`; untrusted out-of-bounds values are reset to 24.
    /// `0` is allowed (explicit opt-in/warning handled by UI/bridge layers).
    pub min_age_hours: u32,
    /// Allow selecting/converting Unknown analyzer directories when explicitly enabled.
    pub allow_delete_unknown: bool,
    /// Validated canonical Custom Path roots (contents-only). Untrusted/missing/unsafe entries are dropped.
    pub custom_paths: Vec<String>,
    /// Project staleness threshold in days for Tier 3 Rust `target/` and `node_modules` discovery.
    /// Bounded to `1..=3650`; untrusted out-of-bounds values are reset to 30.
    #[serde(default = "default_stale_project_days")]
    pub stale_project_days: u32,
}

fn default_stale_project_days() -> u32 {
    DEFAULT_STALE_PROJECT_DAYS
}

impl Default for SweeperSettings {
    fn default() -> Self {
        Self {
            enable_tier1: true,
            enable_tier2: true,
            enable_tier3: false,
            min_age_hours: DEFAULT_MIN_AGE_HOURS,
            allow_delete_unknown: false,
            custom_paths: Vec::new(),
            stale_project_days: DEFAULT_STALE_PROJECT_DAYS,
        }
    }
}

/// Result of parsing untrusted settings JSON with field-level sanitization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedSettings {
    /// Sanitized, safe settings value.
    pub settings: SweeperSettings,
    /// Non-fatal sanitization warnings (e.g. malformed JSON fallback, dropped custom paths).
    pub warnings: Vec<String>,
}

impl SweeperSettings {
    /// Load and sanitize untrusted settings JSON from disk.
    ///
    /// - Missing/unreadable/malformed JSON => safe defaults with a non-fatal warning.
    /// - Each field is sanitized independently.
    /// - Protected-path override fields (if injected) are ignored and never stored.
    pub fn load_untrusted(path: &Path) -> SanitizedSettings {
        match fs::read_to_string(path) {
            Ok(s) => Self::parse_untrusted_json(&s),
            Err(_) => SanitizedSettings {
                settings: Self::default(),
                warnings: vec![
                    "settings file missing or unreadable; using safe defaults".to_string()
                ],
            },
        }
    }

    /// Parse and sanitize untrusted settings JSON text.
    pub fn parse_untrusted_json(text: &str) -> SanitizedSettings {
        let value = match serde_json::from_str::<serde_json::Value>(text) {
            Ok(v) => v,
            Err(_) => {
                return SanitizedSettings {
                    settings: Self::default(),
                    warnings: vec![
                        "settings JSON malformed or corrupt; using safe defaults".to_string()
                    ],
                };
            }
        };
        Self::sanitize_untrusted_value(&value)
    }

    /// Sanitize an untrusted JSON value into validated settings.
    pub fn sanitize_untrusted_value(value: &serde_json::Value) -> SanitizedSettings {
        let mut warnings = Vec::new();
        let mut settings = Self::default();

        let Some(obj) = value.as_object() else {
            warnings.push("settings JSON must be an object; using safe defaults".to_string());
            return SanitizedSettings { settings, warnings };
        };

        extract_bool_field(
            obj,
            &["enable_tier1", "enableTier1"],
            &mut settings.enable_tier1,
            "enable_tier1",
            &mut warnings,
        );
        extract_bool_field(
            obj,
            &["enable_tier2", "enableTier2"],
            &mut settings.enable_tier2,
            "enable_tier2",
            &mut warnings,
        );
        extract_bool_field(
            obj,
            &["enable_tier3", "enableTier3"],
            &mut settings.enable_tier3,
            "enable_tier3",
            &mut warnings,
        );
        extract_u32_field(
            obj,
            U32FieldSpec {
                keys: &["min_age_hours", "minAgeHours"],
                min: 0,
                max: MAX_MIN_AGE_HOURS,
                default: DEFAULT_MIN_AGE_HOURS,
                invalid_name: "min_age_hours",
                out_of_bounds_warning: "min_age_hours out of bounds; reset to default (24h)",
            },
            &mut settings.min_age_hours,
            &mut warnings,
        );
        extract_bool_field(
            obj,
            &["allow_delete_unknown", "allowDeleteUnknown"],
            &mut settings.allow_delete_unknown,
            "allow_delete_unknown",
            &mut warnings,
        );
        extract_u32_field(
            obj,
            U32FieldSpec {
                keys: &["stale_project_days", "staleProjectDays"],
                min: MIN_STALE_PROJECT_DAYS,
                max: MAX_STALE_PROJECT_DAYS,
                default: DEFAULT_STALE_PROJECT_DAYS,
                invalid_name: "stale_project_days",
                out_of_bounds_warning: "stale_project_days out of bounds; reset to default (30d)",
            },
            &mut settings.stale_project_days,
            &mut warnings,
        );
        parse_custom_paths_field(obj, &mut settings, &mut warnings);
        push_protected_override_warning(obj, &mut warnings);

        SanitizedSettings { settings, warnings }
    }
}

/// Validate and sanitize settings-like values in place.
pub fn sanitize_min_age_hours(min_age_hours: u32) -> u32 {
    if min_age_hours > MAX_MIN_AGE_HOURS {
        DEFAULT_MIN_AGE_HOURS
    } else {
        min_age_hours
    }
}

/// Validate settings fields that affect scanning/analysis.
///
/// - `min_age_hours` must be within bounds.
/// - Any custom paths present in settings JSON must pass `policy::validate_custom_path`.
pub fn validate_settings_json<T>(settings: &T) -> Result<(), String>
where
    T: Serialize,
{
    let value = serde_json::to_value(settings).map_err(|e| format!("invalid settings: {e}"))?;
    validate_settings_value(&value)
}

fn validate_settings_value(value: &serde_json::Value) -> Result<(), String> {
    let Some(obj) = value.as_object() else {
        return Err("settings must be an object".to_string());
    };
    if let Some(min_age) = obj.get("min_age_hours").or_else(|| obj.get("minAgeHours")) {
        let Some(n) = min_age.as_u64() else {
            return Err("minimum_age_hours must be a non-negative integer".to_string());
        };
        if n > u64::from(MAX_MIN_AGE_HOURS) {
            return Err("minimum_age_hours is out of bounds".to_string());
        }
    }
    if let Some(v) = obj
        .get("stale_project_days")
        .or_else(|| obj.get("staleProjectDays"))
    {
        let Some(n) = v.as_u64() else {
            return Err("stale_project_days must be a positive integer".to_string());
        };
        if !(u64::from(MIN_STALE_PROJECT_DAYS)..=u64::from(MAX_STALE_PROJECT_DAYS)).contains(&n) {
            return Err("stale_project_days is out of bounds".to_string());
        }
    }
    for key in ["custom_paths", "customPaths"] {
        if let Some(paths) = obj.get(key) {
            validate_custom_paths_value(paths)?;
        }
    }
    Ok(())
}

fn validate_custom_paths_value(value: &serde_json::Value) -> Result<(), String> {
    let Some(arr) = value.as_array() else {
        return Err("custom paths must be an array".to_string());
    };
    let mut seen: HashSet<String> = HashSet::new();
    for p in arr {
        let Some(s) = p.as_str() else {
            return Err("custom path entries must be strings".to_string());
        };
        let verdict = crate::policy::validate_custom_path(s);
        if !verdict.allowed {
            return Err(format!("custom path not allowed: {}", verdict.reason));
        }
        let canon = CanonicalPath::parse_lexical(s)
            .map_err(|e| format!("custom path not allowed: {e}"))?
            .as_str()
            .to_ascii_uppercase();
        if !seen.insert(canon) {
            return Err("custom path entries must be unique".to_string());
        }
    }
    Ok(())
}

fn get_field<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|k| obj.get(*k))
}

fn extract_bool_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
    target: &mut bool,
    field_name: &str,
    warnings: &mut Vec<String>,
) {
    let Some(v) = get_field(obj, keys) else {
        return;
    };
    match v.as_bool() {
        Some(b) => *target = b,
        None => warnings.push(format!("{field_name} invalid; using default")),
    }
}

struct U32FieldSpec<'a> {
    keys: &'a [&'a str],
    min: u32,
    max: u32,
    default: u32,
    invalid_name: &'a str,
    out_of_bounds_warning: &'a str,
}

fn extract_u32_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    spec: U32FieldSpec<'_>,
    target: &mut u32,
    warnings: &mut Vec<String>,
) {
    let Some(v) = get_field(obj, spec.keys) else {
        return;
    };
    match v.as_u64() {
        Some(n) if (u64::from(spec.min)..=u64::from(spec.max)).contains(&n) => *target = n as u32,
        Some(_) => {
            *target = spec.default;
            warnings.push(spec.out_of_bounds_warning.to_string());
        }
        None => warnings.push(format!("{} invalid; using default", spec.invalid_name)),
    }
}

fn parse_custom_paths_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    settings: &mut SweeperSettings,
    warnings: &mut Vec<String>,
) {
    let Some(v) = get_field(obj, &["custom_paths", "customPaths"]) else {
        return;
    };
    settings.custom_paths = sanitize_custom_paths_value(v, warnings);
}

fn push_protected_override_warning(
    obj: &serde_json::Map<String, serde_json::Value>,
    warnings: &mut Vec<String>,
) {
    if obj.contains_key("protected_paths")
        || obj.contains_key("protectedPaths")
        || obj.contains_key("exclusions")
    {
        warnings.push(
            "protected path/exclusion overrides are ignored (not configurable via settings)"
                .to_string(),
        );
    }
}

fn try_normalize_custom_path(s: &str, seen: &mut HashSet<String>) -> Result<String, String> {
    let verdict = crate::policy::validate_custom_path(s);
    if !verdict.allowed {
        return Err(format!("custom path dropped: {}", verdict.reason));
    }
    let canon = CanonicalPath::parse_lexical(s)
        .map_err(|_| "custom path dropped: invalid path".to_string())?
        .as_str()
        .to_string();
    let key = canon.to_ascii_uppercase();
    if !seen.insert(key) {
        return Err("custom path dropped: duplicate".to_string());
    }
    Ok(canon)
}

fn sanitize_custom_paths_value(
    value: &serde_json::Value,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let Some(arr) = value.as_array() else {
        warnings.push("custom_paths invalid; expected array (dropped)".to_string());
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for p in arr {
        let Some(s) = p.as_str() else {
            warnings.push("custom path entry dropped: not a string".to_string());
            continue;
        };
        match try_normalize_custom_path(s, &mut seen) {
            Ok(canon) => out.push(canon),
            Err(msg) => warnings.push(msg),
        }
    }
    out
}

/// Persist settings JSON to disk safely (creates parent directories).
///
/// Writes directly to the target path to avoid orphaned temp files and to remain
/// compatible with gate-a rules (no remove/unlink in production backend code).
pub fn persist_settings_json<T>(path: &Path, settings: &T) -> Result<(), String>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create settings dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("failed to serialize settings: {e}"))?;
    fs::write(path, json.as_bytes()).map_err(|e| format!("failed to write settings: {e}"))
}

/// Load settings JSON from disk.
///
/// For `SweeperSettings`, this performs untrusted parsing with field-level sanitization and
/// safe-default fallback (never panics on malformed/tampered input). For other types, it
/// preserves the legacy strict behavior (deserialize + validate, else `None`).
pub fn load_settings_json<T>(path: &Path) -> Option<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let s = fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<T>(&s).ok()?;
    if validate_settings_json(&parsed).is_err() {
        return None;
    }
    Some(parsed)
}

/// Load untrusted `SweeperSettings` from disk with sanitization and non-fatal warnings.
pub fn load_untrusted_settings(path: &Path) -> SanitizedSettings {
    SweeperSettings::load_untrusted(path)
}

/// Parse untrusted `SweeperSettings` JSON with sanitization and non-fatal warnings.
pub fn parse_untrusted_settings_json(text: &str) -> SanitizedSettings {
    SweeperSettings::parse_untrusted_json(text)
}
