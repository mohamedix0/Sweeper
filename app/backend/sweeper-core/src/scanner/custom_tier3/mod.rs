use super::*;
use sweeper_winfs::KnownFolderId;

mod custom_paths;
mod dir_summary;
mod recycle_bin;
mod stale_policy;
mod stale_projects;

#[allow(unused_imports)]
pub(in crate::scanner) use dir_summary::summarize_directory_for_scan;

pub(super) struct CustomScanCtx<'a> {
    pub(super) min_age: Duration,
    pub(super) now: SystemTime,
    pub(super) progress: &'a mut ScanProgressRuntime,
}

pub(super) fn append_custom_path_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<(), ScanError> {
    let mut ctx = CustomScanCtx {
        min_age,
        now,
        progress,
    };
    custom_paths::append_custom_path_targets(report, settings, &mut ctx)
}

pub(super) fn append_tier3_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
    progress: &mut ScanProgressRuntime,
) -> Result<(), ScanError> {
    let mut ctx = CustomScanCtx {
        min_age,
        now,
        progress,
    };
    recycle_bin::append_tier3_targets(report, settings, opts, &mut ctx)
}
