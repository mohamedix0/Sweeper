use super::*;

mod policy;
mod roots;
mod targets;
mod temp_guard;
mod wildcard;

pub use temp_guard::evaluate_redirected_temp_candidate;

pub(in crate::scanner) use policy::{paths_overlap, validate_resolved_canonical_root};
pub(in crate::scanner) use roots::path_has_link_point_ancestor;
pub(in crate::scanner) use targets::{scan_resolved_roots, ResolvedRootsScanCtx};
pub(in crate::scanner) use temp_guard::filetime_to_unix_secs;

/// Bundled input for allowlisted scan runtime execution.
pub(super) struct AllowlistedScanRuntimeInput<'a, 'p> {
    pub targets: Vec<TargetDefinition>,
    pub min_age_hours: u32,
    pub redirected_temp_candidate: Option<&'a str>,
    pub elevation_target_root_overrides: &'a BTreeMap<String, String>,
    pub progress: &'p mut ScanProgressRuntime,
    pub started_unix_secs: u64,
}

pub(super) struct AllowlistedScanCtx<'a> {
    pub(super) now: SystemTime,
    pub(super) min_age: Duration,
    pub(super) redirected_temp_candidate: Option<&'a str>,
}

impl<'a> AllowlistedScanCtx<'a> {
    pub(super) fn new(
        started_unix_secs: u64,
        min_age_hours: u32,
        redirected_temp_candidate: Option<&'a str>,
    ) -> Self {
        Self {
            now: UNIX_EPOCH + Duration::from_secs(started_unix_secs),
            min_age: Duration::from_secs(u64::from(min_age_hours) * 3600),
            redirected_temp_candidate,
        }
    }
}

pub(super) struct AllowlistedScanEnv<'a, 'p> {
    pub(super) kf_roots: &'a BTreeMap<KnownFolder, Result<CanonicalPath, String>>,
    pub(super) elevation_target_root_overrides: &'a BTreeMap<String, String>,
    pub(super) ctx: &'a AllowlistedScanCtx<'a>,
    pub(super) progress: &'p mut ScanProgressRuntime,
}

pub(super) fn scan_allowlisted_targets_with_progress_runtime(
    input: AllowlistedScanRuntimeInput<'_, '_>,
) -> Result<ScanReport, ScanError> {
    let AllowlistedScanRuntimeInput {
        targets,
        min_age_hours,
        redirected_temp_candidate,
        elevation_target_root_overrides,
        progress,
        started_unix_secs,
    } = input;
    validate_target_definitions(&targets).map_err(ScanError::Targets)?;
    let ctx = AllowlistedScanCtx::new(started_unix_secs, min_age_hours, redirected_temp_candidate);
    let kf_roots =
        roots::resolve_needed_known_folder_roots(&targets, elevation_target_root_overrides);
    let mut env = AllowlistedScanEnv {
        kf_roots: &kf_roots,
        elevation_target_root_overrides,
        ctx: &ctx,
        progress,
    };
    let results = targets::scan_allowlisted_target_list(targets, &mut env)?;
    Ok(ScanReport {
        started_unix_secs,
        min_age_hours,
        targets: results,
    })
}

pub(super) fn system_time_to_unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn validate_target_definitions(targets: &[TargetDefinition]) -> Result<(), String> {
    let lint_targets: Vec<sweeper_knowledge::LintTarget> = targets
        .iter()
        .map(super::target_definition_to_lint_target)
        .collect();
    sweeper_knowledge::lint_targets(&lint_targets).map_err(|e| e.to_string())
}
