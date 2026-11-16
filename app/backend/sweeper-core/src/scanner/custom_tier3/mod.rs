use super::*;
use sweeper_winfs::KnownFolderId;

mod custom_paths;
mod dir_summary;
mod recycle_bin;
mod stale_policy;
mod stale_projects;

#[allow(unused_imports)]
pub(in crate::scanner) use dir_summary::summarize_directory_for_scan;

pub(super) struct Tier3RunCtx<'a, 'p> {
    pub(super) settings: &'a crate::settings::SweeperSettings,
    pub(super) opts: &'a ScanSettingsOptions,
    pub(super) min_age: Duration,
    pub(super) now: SystemTime,
    pub(super) progress: &'p mut ScanProgressRuntime,
}

pub(super) fn append_custom_path_targets(
    report: &mut ScanReport,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<(), ScanError> {
    custom_paths::append_custom_path_targets(report, ctx)
}

pub(super) fn append_tier3_targets(
    report: &mut ScanReport,
    ctx: &mut Tier3RunCtx<'_, '_>,
) -> Result<(), ScanError> {
    recycle_bin::append_tier3_targets(report, ctx)
}
