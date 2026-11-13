//! Settings validation and persistence helpers.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Maximum allowed `minimum_age_hours` (10 years).
pub const MAX_MIN_AGE_HOURS: u32 = 24 * 365 * 10;

/// Validate and sanitize settings-like values in place.
pub fn sanitize_min_age_hours(min_age_hours: u32) -> u32 {
    if min_age_hours > MAX_MIN_AGE_HOURS {
        24
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
    for p in arr {
        let Some(s) = p.as_str() else {
            return Err("custom path entries must be strings".to_string());
        };
        let verdict = crate::policy::validate_custom_path(s);
        if !verdict.allowed {
            return Err(format!("custom path not allowed: {}", verdict.reason));
        }
    }
    Ok(())
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

/// Load settings JSON from disk, returning `None` on missing/invalid/unreadable input.
pub fn load_settings_json<T>(path: &Path) -> Option<T>
where
    T: for<'de> Deserialize<'de> + Serialize,
{
    let s = fs::read_to_string(path).ok()?;
    let parsed = serde_json::from_str::<T>(&s).ok()?;
    validate_settings_json(&parsed).ok()?;
    Some(parsed)
}
