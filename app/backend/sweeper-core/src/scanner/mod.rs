use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg_attr(not(test), allow(unused_imports))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::{KnownFolder, Risk, TargetDefinition, Tier};
use sweeper_winfs::{self, DirEntry, FileIdentity, KnownFolderId, ReparseTagClass, WinfsError};

use crate::paths::CanonicalPath;
use crate::policy;

mod allowlist;
mod custom_tier3;
mod gate_e;
mod report;
mod runtime;
#[cfg(test)]
mod tests;
mod walk;

pub use allowlist::evaluate_redirected_temp_candidate;
pub use gate_e::gate_e_violations;
pub use report::format_report;
pub(in crate::scanner) use runtime::ScanProgressRuntime;

/// Reasons an entry is skipped during scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkipReason {
    /// Symlink or junction (`Link Point`).
    LinkPoint,
    /// Cloud placeholder (e.g. OneDrive/cldflt). Never hydrated.
    CloudPlaceholder,
    /// Other reparse tag.
    OtherReparse,
    /// Unreadable/inaccessible entry or metadata.
    Unreadable,
}

impl SkipReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::LinkPoint => "LinkPoint",
            Self::CloudPlaceholder => "CloudPlaceholder",
            Self::OtherReparse => "OtherReparse",
            Self::Unreadable => "Unreadable",
        }
    }
}

/// Result of redirected `%TEMP%` guard evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TempGuard {
    /// Use the provided canonical redirected temp path.
    Use(CanonicalPath),
    /// Redirected temp must be auto-disabled for safety/redundancy.
    Disabled(String),
}

/// Metadata for a scannable/observed item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanItem {
    /// Stable per-scan item identifier.
    pub id: String,
    /// Owning target id.
    pub target_id: String,
    /// Canonical/resolved path string (display/internal; never submitted for deletion IPC in this crate).
    pub path: String,
    /// File identity (volume serial + file index) when available.
    pub file_identity: Option<FileIdentity>,
    /// Logical size in bytes.
    pub size: u64,
    /// Last modification time (system time seconds since UNIX epoch, saturating).
    pub mtime_unix_secs: u64,
    /// Whether the item is eligible by minimum-age policy (and not future-dated).
    pub age_eligible: bool,
}

/// Per-target scan totals and status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetScanResult {
    /// Target definition id.
    pub target_id: String,
    /// Target name.
    pub target_name: String,
    /// Tier.
    pub tier: Tier,
    /// Risk.
    pub risk: Risk,
    /// Resolved concrete root paths that were scanned (expanded from wildcards).
    pub resolved_paths: Vec<String>,
    /// Whether the target was scanned.
    pub scanned: bool,
    /// Disabled/error/status reason when not scanned or partially disabled.
    pub status_reason: Option<String>,
    /// Total bytes observed.
    pub total_bytes: u64,
    /// Eligible bytes observed.
    pub eligible_bytes: u64,
    /// Total file/item count observed.
    pub file_count: u64,
    /// Eligible file/item count observed.
    pub eligible_count: u64,
    /// Skipped entry count.
    pub skipped_count: u64,
    /// Skipped reason counts.
    pub skipped_reasons: BTreeMap<String, u64>,
    /// Detailed skip diagnostics (e.g. unreadable path/error context) for Gate E/reporting.
    #[serde(default)]
    pub skipped_diagnostics: Vec<String>,
    /// Per-item metadata.
    pub items: Vec<ScanItem>,
}

/// Aggregate scan report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    /// When the scan started (unix seconds).
    pub started_unix_secs: u64,
    /// Minimum age in hours applied.
    pub min_age_hours: u32,
    /// Per-target results.
    pub targets: Vec<TargetScanResult>,
}

/// Read-only incremental scan progress snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgress {
    /// Number of targets completed so far.
    pub completed_targets: u32,
    /// Total number of targets in this scan run.
    pub total_targets: u32,
    /// Currently scanning target id, if any.
    pub current_target_id: Option<String>,
    /// Currently scanning target name, if any.
    pub current_target_name: Option<String>,
    /// Aggregate scanned/observed items so far.
    pub scanned_items: u64,
    /// Aggregate scanned/observed bytes so far.
    pub scanned_bytes: u64,
}

/// Cooperative scan progress callback.
pub type ScanProgressCallback = Arc<dyn Fn(ScanProgress) + Send + Sync>;

/// Scan-time errors that are not per-target disable reasons.
#[derive(Debug, Error)]
pub enum ScanError {
    /// Failed to enumerate embedded allowlist.
    #[error("failed to load embedded targets: {0}")]
    Targets(String),
    /// Scan was cooperatively cancelled.
    #[error("scan cancelled")]
    Cancelled,
}

/// Scan enabled Tier 1 & Tier 2 embedded targets with the given minimum-age override (hours).
pub fn scan_tier1_tier2(min_age_hours: u32) -> Result<ScanReport, ScanError> {
    let targets = sweeper_knowledge::embedded_targets();
    scan_targets(
        targets
            .into_iter()
            .filter(|t| matches!(t.tier, Tier::Tier1 | Tier::Tier2))
            .collect(),
        min_age_hours,
    )
}

/// Scan a provided set of targets (read-only).
pub fn scan_targets(
    targets: Vec<TargetDefinition>,
    min_age_hours: u32,
) -> Result<ScanReport, ScanError> {
    scan_targets_with_redirected_temp_override(targets, min_age_hours, None)
}

/// Scan a provided set of targets (read-only) with an optional injected redirected `%TEMP%`
/// candidate path for deterministic/testing scenarios.
///
/// When `redirected_temp_candidate` is `Some`, it is used instead of reading process `%TEMP%`
/// for `redirected-temp` guard evaluation.
pub fn scan_targets_with_redirected_temp_override(
    targets: Vec<TargetDefinition>,
    min_age_hours: u32,
    redirected_temp_candidate: Option<&str>,
) -> Result<ScanReport, ScanError> {
    scan_targets_with_redirected_temp_and_elevation_overrides(
        targets,
        min_age_hours,
        redirected_temp_candidate,
        &BTreeMap::new(),
    )
}

/// Options for allowlisted target scans with progress/cancellation support.
#[derive(Clone)]
pub struct AllowlistedScanControl<'a> {
    /// Minimum age in hours.
    pub min_age_hours: u32,
    /// Optional injected redirected `%TEMP%` candidate.
    pub redirected_temp_candidate: Option<&'a str>,
    /// Optional root overrides for audited elevation targets.
    pub elevation_target_root_overrides: &'a BTreeMap<String, String>,
    /// Cooperative cancellation flag.
    pub cancel: Arc<AtomicBool>,
    /// Optional progress callback.
    pub progress_cb: Option<ScanProgressCallback>,
}

/// Scan a provided set of targets with optional redirected-temp and elevation-target root overrides.
///
/// Elevation overrides are only applied to `requires_elevation` targets and are validated with the
/// same safety checks as normal resolved roots (non-bypassing test/harness seam).
pub fn scan_targets_with_redirected_temp_and_elevation_overrides(
    targets: Vec<TargetDefinition>,
    min_age_hours: u32,
    redirected_temp_candidate: Option<&str>,
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> Result<ScanReport, ScanError> {
    scan_targets_with_redirected_temp_and_elevation_overrides_with_progress(
        targets,
        AllowlistedScanControl {
            min_age_hours,
            redirected_temp_candidate,
            elevation_target_root_overrides,
            cancel: Arc::new(AtomicBool::new(false)),
            progress_cb: None,
        },
    )
}

/// Scan a provided set of targets with optional overrides and cooperative cancellation/progress.
pub fn scan_targets_with_redirected_temp_and_elevation_overrides_with_progress(
    targets: Vec<TargetDefinition>,
    control: AllowlistedScanControl<'_>,
) -> Result<ScanReport, ScanError> {
    let mut progress =
        ScanProgressRuntime::new(targets.len() as u32, control.cancel, control.progress_cb);
    let started_unix_secs = allowlist::system_time_to_unix_secs(SystemTime::now());
    allowlist::scan_allowlisted_targets_with_progress_runtime(
        allowlist::AllowlistedScanRuntimeInput {
            targets,
            min_age_hours: control.min_age_hours,
            redirected_temp_candidate: control.redirected_temp_candidate,
            elevation_target_root_overrides: control.elevation_target_root_overrides,
            progress: &mut progress,
            started_unix_secs,
        },
    )
}

#[derive(Debug, Clone)]
pub(super) struct TargetResultMeta {
    pub(super) target_id: String,
    pub(super) target_name: String,
    pub(super) tier: Tier,
    pub(super) risk: Risk,
}

/// Options for settings-driven scans (custom paths and Tier 3 conditionals).
#[derive(Debug, Clone, Default)]
pub struct ScanSettingsOptions {
    /// Override current time (tests). Uses `SystemTime::now()` when `None`.
    pub now: Option<SystemTime>,
    /// Optional injected redirected `%TEMP%` candidate path for deterministic tests.
    pub redirected_temp_candidate: Option<String>,
    /// Optional injected recycle-bin scan roots (directories whose contents are scanned as Tier 3 recycle-bin).
    /// When empty and Tier 3 recycle-bin scan is requested in production, accessible `$Recycle.Bin\<SID>` dirs are discovered.
    pub recycle_bin_roots: Vec<String>,
    /// Optional injected project search roots for Tier 3 stale `target/` and `node_modules` discovery.
    /// When empty in production opt-in scans, a conservative default (user profile root) is used if available.
    pub tier3_project_search_roots: Vec<String>,
    /// Optional test/harness-only root overrides for audited elevation targets.
    ///
    /// Keys are target ids (e.g. `windows-temp`). When present for a `requires_elevation`
    /// target, the override path is canonicalized and validated with the same safety checks as
    /// any resolved target root before scanning; this enables isolated sentinel-sandbox tests
    /// without touching real system paths.
    pub elevation_target_root_overrides: BTreeMap<String, String>,
}

/// Scan Tier 1/2 allowlisted targets plus validated Custom Paths and opt-in Tier 3 conditional targets.
///
/// - Tier toggles are respected (`enable_tier1/2/3`).
/// - Custom Paths are re-validated at scan time; missing/unsafe paths emit zero items and diagnostics (never fallback to parent).
/// - Tier 3 targets (`recycle-bin`, `stale-rust-target`, `stale-node-modules`) run only when `enable_tier3` is true.
pub fn scan_with_settings(
    settings: &crate::settings::SweeperSettings,
) -> Result<ScanReport, ScanError> {
    scan_with_settings_options(settings, ScanSettingsOptions::default())
}

/// Scan with injectable options for deterministic/testable Tier 3 and custom-path behavior.
pub fn scan_with_settings_options(
    settings: &crate::settings::SweeperSettings,
    opts: ScanSettingsOptions,
) -> Result<ScanReport, ScanError> {
    scan_with_settings_options_and_elevation_with_progress(
        settings,
        opts,
        crate::ScanElevationContext::default(),
        ScanRunControl::default(),
    )
}

/// Scan using the current process elevation state (via `sweeper_winfs::is_process_elevated()`).
pub fn scan_with_settings_current_elevation(
    settings: &crate::settings::SweeperSettings,
) -> Result<ScanReport, ScanError> {
    scan_with_settings_options_current_elevation_with_progress(
        settings,
        ScanSettingsOptions::default(),
        ScanRunControl::default(),
    )
}

/// Cooperative scan execution control (cancellation + progress).
#[derive(Clone, Default)]
pub struct ScanRunControl {
    /// Cooperative cancellation flag.
    pub cancel: Arc<AtomicBool>,
    /// Optional progress callback.
    pub progress_cb: Option<ScanProgressCallback>,
}

/// Scan with explicit elevation context and cooperative cancellation/progress reporting.
pub fn scan_with_settings_options_and_elevation_with_progress(
    settings: &crate::settings::SweeperSettings,
    opts: ScanSettingsOptions,
    elevation: crate::ScanElevationContext,
    control: ScanRunControl,
) -> Result<ScanReport, ScanError> {
    let now = opts.now.unwrap_or_else(SystemTime::now);
    let started_unix_secs = allowlist::system_time_to_unix_secs(now);
    let min_age = Duration::from_secs(u64::from(settings.min_age_hours) * 3600);

    let mut elev_ok = elevation.process_elevated;
    if elev_ok {
        elev_ok = settings.enable_tier3;
    }
    if elev_ok {
        elev_ok = settings.enable_elevation_targets;
    }
    let allowlisted_targets: Vec<TargetDefinition> = sweeper_knowledge::embedded_targets()
        .into_iter()
        .filter(|t| {
            if t.requires_elevation {
                elev_ok
            } else {
                match t.tier {
                    Tier::Tier1 => settings.enable_tier1,
                    Tier::Tier2 => settings.enable_tier2,
                    Tier::Tier3 => settings.enable_tier3,
                }
            }
        })
        .collect();

    let tier3_append_count = if settings.enable_tier3 {
        1 /* recycle-bin */ + 2 /* stale rust target + stale node_modules */
    } else {
        0
    };
    let total_targets =
        (allowlisted_targets.len() + settings.custom_paths.len() + tier3_append_count) as u32;
    let mut progress = ScanProgressRuntime::new(total_targets, control.cancel, control.progress_cb);

    let mut report = if allowlisted_targets.is_empty() {
        ScanReport {
            started_unix_secs,
            min_age_hours: settings.min_age_hours,
            targets: Vec::new(),
        }
    } else {
        allowlist::scan_allowlisted_targets_with_progress_runtime(
            allowlist::AllowlistedScanRuntimeInput {
                targets: allowlisted_targets,
                min_age_hours: settings.min_age_hours,
                redirected_temp_candidate: opts.redirected_temp_candidate.as_deref(),
                elevation_target_root_overrides: &opts.elevation_target_root_overrides,
                progress: &mut progress,
                started_unix_secs,
            },
        )?
    };
    report.started_unix_secs = started_unix_secs;
    report.min_age_hours = settings.min_age_hours;

    let mut tier3_ctx = custom_tier3::Tier3RunCtx {
        settings,
        opts: &opts,
        min_age,
        now,
        progress: &mut progress,
    };
    custom_tier3::append_custom_path_targets(&mut report, &mut tier3_ctx)?;
    custom_tier3::append_tier3_targets(&mut report, &mut tier3_ctx)?;

    Ok(report)
}

/// Scan with options using the current process elevation state and cooperative cancellation/progress.
pub fn scan_with_settings_options_current_elevation_with_progress(
    settings: &crate::settings::SweeperSettings,
    opts: ScanSettingsOptions,
    control: ScanRunControl,
) -> Result<ScanReport, ScanError> {
    let elevated = sweeper_winfs::is_process_elevated().unwrap_or(false);
    scan_with_settings_options_and_elevation_with_progress(
        settings,
        opts,
        crate::ScanElevationContext {
            process_elevated: elevated,
        },
        control,
    )
}

pub(super) fn empty_target_result(meta: TargetResultMeta) -> TargetScanResult {
    TargetScanResult {
        target_id: meta.target_id,
        target_name: meta.target_name,
        tier: meta.tier,
        risk: meta.risk,
        resolved_paths: Vec::new(),
        scanned: false,
        status_reason: None,
        total_bytes: 0,
        eligible_bytes: 0,
        file_count: 0,
        eligible_count: 0,
        skipped_count: 0,
        skipped_reasons: BTreeMap::new(),
        skipped_diagnostics: Vec::new(),
        items: Vec::new(),
    }
}

fn target_definition_to_lint_target(t: &TargetDefinition) -> sweeper_knowledge::LintTarget {
    sweeper_knowledge::LintTarget {
        id: t.id.clone(),
        name: t.name.clone(),
        tier: match t.tier {
            Tier::Tier1 => "Tier1".to_string(),
            Tier::Tier2 => "Tier2".to_string(),
            Tier::Tier3 => "Tier3".to_string(),
        },
        risk: match t.risk {
            Risk::Safe => "Safe".to_string(),
            Risk::Caution => "Caution".to_string(),
            Risk::Unknown => "Unknown".to_string(),
            Risk::Protected => "Protected".to_string(),
        },
        known_folder: match t.known_folder {
            KnownFolder::LocalAppData => "LocalAppData".to_string(),
            KnownFolder::RoamingAppData => "RoamingAppData".to_string(),
            KnownFolder::UserProfile => "UserProfile".to_string(),
            KnownFolder::Windows => "Windows".to_string(),
            KnownFolder::ProgramData => "ProgramData".to_string(),
        },
        relative_subpath: t.relative_subpath.clone(),
        scope: "contents".to_string(),
        min_age_hours: t.min_age_hours,
        description: t.description.clone(),
        requires_elevation: t.requires_elevation,
    }
}
