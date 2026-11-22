use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sweeper_contract::{
    ContractError, ExplainItemResponse, PreviewCleanResponse, PreviewConfirmationsDto,
    ScanProgressEvent, ScanResultsDto, StartScanResponse, TierDto,
};
use sweeper_core::scanner::{self, ScanError};

use crate::clock::Clock;
use crate::session::{build_prepared_scan_session, ScanFinishInput};
use crate::AppState;

use super::shared::{
    build_scan_progress_cb, commit_async_outcome, emit_scan_event, lock_state_error,
    AsyncOutcomeCommit, CommitStartGenerations, SharedScanCb,
};

#[derive(Clone, Copy)]
struct ScanRunCtx<'a, F>
where
    F: FnMut(ScanProgressEvent),
{
    session_id: &'a str,
    prepared: &'a crate::session::PreparedScan,
    shared_cb: &'a Option<SharedScanCb<F>>,
}

fn commit_scan_outcome<C: Clock>(
    state: &AppState<C>,
    session_id: &str,
    prepared: &crate::session::PreparedScan,
    report: scanner::ScanReport,
) -> Result<(), ContractError> {
    // Build the large DTO tree while no global lock is held; the commit below
    // only inserts the finished tree under the short store lock (#96).
    let prepared_tree = build_prepared_scan_session(session_id, report);
    commit_async_outcome(
        state,
        AsyncOutcomeCommit::new(
            session_id.to_string(),
            CommitStartGenerations {
                scan: prepared.settings_generation,
                // Scan commits validate only the scan generation; the analyzer
                // generation is unused here but carried by the shared commit seam.
                analyzer: 0,
            },
            (prepared.effective_settings.min_age_hours, prepared_tree),
            |clock: &C,
             sessions: &mut crate::session::SessionStore,
             sid: String,
             gens: super::shared::CommitGenerations,
             payload: (u32, crate::session::PreparedScanSession)| {
                sessions.finish_scan(
                    clock,
                    ScanFinishInput {
                        session_id: sid,
                        settings_generation_at_start: gens.scan_at_start,
                        current_settings_generation: gens.scan_current,
                        effective_min_age_hours: payload.0,
                        prepared: payload.1,
                    },
                )
            },
        ),
    )
}

fn has_any_builtin_tier_enabled(settings: &sweeper_core::settings::SweeperSettings) -> bool {
    if settings.enable_tier1 || settings.enable_tier2 {
        return true;
    }
    settings.enable_tier3
}

fn custom_path_is_verified_sandbox(path: &str) -> bool {
    if super::is_sandbox_path_hint(path) {
        return true;
    }
    std::path::Path::new(path)
        .join(".sweeper_sandbox_sentinel")
        .exists()
}

fn path_is_within_verified_custom_sandbox(
    settings: &sweeper_core::settings::SweeperSettings,
    path: &str,
) -> bool {
    let candidate = std::path::Path::new(path);
    settings.custom_paths.iter().any(|custom| {
        if !custom_path_is_verified_sandbox(custom) {
            return false;
        }
        candidate.starts_with(custom)
    })
}

fn all_custom_paths_are_sandbox(settings: &sweeper_core::settings::SweeperSettings) -> bool {
    settings
        .custom_paths
        .iter()
        .all(|p| custom_path_is_verified_sandbox(p))
}

fn report_targets_are_sandbox_custom(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
) -> bool {
    report.targets.iter().all(|t| {
        if !t.target_id.starts_with("custom-path") {
            return false;
        }
        t.resolved_paths.iter().all(|p| {
            super::is_sandbox_path_hint(p) || path_is_within_verified_custom_sandbox(settings, p)
        })
    })
}

fn is_sandbox_scan(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
) -> bool {
    if has_any_builtin_tier_enabled(settings) {
        return false;
    }
    if settings.custom_paths.is_empty() {
        return false;
    }
    if !all_custom_paths_are_sandbox(settings) {
        return false;
    }
    report_targets_are_sandbox_custom(settings, report)
}

fn scan_logs_base_dir() -> PathBuf {
    std::env::var_os("SWEEPER_LOGS_BASE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(sweeper_core::journal::default_logs_base_dir)
}

/// Borrowed per-target fields used to assemble the scan activity-log record.
///
/// Every string field is a reference into the source [`scanner::ScanReport`]
/// and every count is a primitive. Building this view is O(number of targets),
/// never O(number of items), and carrying only borrows makes a deep clone of
/// the (potentially huge) report structurally impossible along this path
/// (#96). The focused unit test below asserts the reference identity of these
/// borrows so reintroducing a copy fails loudly.
#[derive(Debug)]
struct BorrowedLogTarget<'a> {
    target_id: &'a str,
    target_name: &'a str,
    tier: sweeper_core::Tier,
    risk: sweeper_core::Risk,
    eligible_bytes: u64,
    total_bytes: u64,
    eligible_count: u64,
    file_count: u64,
    skipped_count: u64,
}

fn borrow_log_targets(report: &scanner::ScanReport) -> Vec<BorrowedLogTarget<'_>> {
    report
        .targets
        .iter()
        .map(|t| BorrowedLogTarget {
            target_id: &t.target_id,
            target_name: &t.target_name,
            tier: t.tier,
            risk: t.risk,
            eligible_bytes: t.eligible_bytes,
            total_bytes: t.total_bytes,
            eligible_count: t.eligible_count,
            file_count: t.file_count,
            skipped_count: t.skipped_count,
        })
        .collect()
}

/// Build the scan activity-log record from a borrow of the report (#96).
///
/// Only the small JSON record is materialised here; the full `ScanReport` is
/// never cloned. Returns `None` for synthetic sandbox scans, which are not
/// logged (same rule the previous in-place logger applied).
fn build_scan_activity_log_record(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
) -> Option<serde_json::Value> {
    build_scan_activity_log_record_observed(settings, report, |_| {})
}

/// Instrumented core of [`build_scan_activity_log_record`]. The `observe`
/// callback receives the borrowed target view built from the live report. The
/// focused unit test uses it to assert, by reference identity, that the record
/// is derived from borrows of the source report and never from a deep copy
/// (#96). Production passes a no-op observer.
fn build_scan_activity_log_record_observed<F>(
    settings: &sweeper_core::settings::SweeperSettings,
    report: &scanner::ScanReport,
    observe: F,
) -> Option<serde_json::Value>
where
    F: FnOnce(&[BorrowedLogTarget<'_>]),
{
    if is_sandbox_scan(settings, report) {
        return None;
    }
    // Materialise only a borrowed view of the targets — never owned item data.
    let borrowed = borrow_log_targets(report);
    observe(&borrowed);
    // Serialized log aggregates must saturate rather than overflow/wrap.
    let total_reclaimable_bytes =
        sweeper_core::journal::saturating_sum_u64(borrowed.iter().map(|t| t.eligible_bytes));
    let total_eligible_files =
        sweeper_core::journal::saturating_sum_u64(borrowed.iter().map(|t| t.eligible_count));
    let total_skipped_files =
        sweeper_core::journal::saturating_sum_u64(borrowed.iter().map(|t| t.skipped_count));
    let categories: Vec<serde_json::Value> = borrowed
        .into_iter()
        .filter(|t| t.eligible_count > 0 || t.file_count > 0)
        .map(|t| {
            serde_json::json!({
                "target_id": t.target_id,
                "target_name": t.target_name,
                "tier": format!("{:?}", t.tier),
                "risk": format!("{:?}", t.risk),
                "eligible_bytes": t.eligible_bytes.to_string(),
                "total_bytes": t.total_bytes.to_string(),
                "eligible_count": t.eligible_count.to_string(),
                "file_count": t.file_count.to_string(),
                "skipped_count": t.skipped_count.to_string(),
            })
        })
        .collect();
    Some(serde_json::json!({
        "kind": "scan",
        "record": "outcome",
        "event": "scan",
        "timestamp_unix_secs": report.started_unix_secs,
        "total_reclaimable_bytes": total_reclaimable_bytes.to_string(),
        "eligible_bytes": total_reclaimable_bytes.to_string(),
        "total_eligible_files": total_eligible_files.to_string(),
        "total_skipped_files": total_skipped_files.to_string(),
        "categories_count": categories.len(),
        "targets": categories,
    }))
}

/// Write an already-built scan activity-log record and surface retirement/IO
/// warnings. Kept separate from record construction so the report can be
/// borrowed and released before the long-lived commit path takes ownership.
fn write_scan_log_record(record: serde_json::Value) {
    match sweeper_core::journal::write_scan_log(&scan_logs_base_dir(), &record) {
        Ok(outcome) => {
            for warning in outcome.retirement_warnings {
                eprintln!("scan activity log retirement warning: {warning}");
            }
        }
        Err(err) => eprintln!("failed to write scan activity log: {err}"),
    }
}

enum ScanTerminal {
    Cancelled,
    Failed(ContractError),
}

fn handle_scan_terminal<C: Clock, F>(
    state: &AppState<C>,
    ctx: ScanRunCtx<'_, F>,
    outcome: ScanTerminal,
) -> ContractError
where
    F: FnMut(ScanProgressEvent),
{
    if let Ok(mut sessions) = state.sessions.lock() {
        sessions.abort_scan(ctx.session_id);
    }
    let (event, err) = match outcome {
        ScanTerminal::Cancelled => (
            ScanProgressEvent::Cancelled {
                session_id: ctx.session_id.to_string(),
            },
            ContractError::ScanCancelled {
                session_id: ctx.session_id.to_string(),
            },
        ),
        ScanTerminal::Failed(err) => (
            ScanProgressEvent::Failed {
                session_id: ctx.session_id.to_string(),
                error: err.clone(),
            },
            err,
        ),
    };
    emit_scan_event(ctx.shared_cb, event);
    err
}

/// `start_scan` implementation.
///
/// Performs staged locking: read settings and register/pre-allocate session under short locks,
/// runs scan without holding `SessionStore`/settings locks, then commits session under short lock.
/// Emits `ScanProgressEvent` (`Started`, `Progress`, `Cancelled|Completed|Failed`) when `on_progress` is provided.
pub fn start_scan<C, F>(
    state: &AppState<C>,
    tiers: Option<Vec<TierDto>>,
    on_progress: Option<F>,
) -> Result<StartScanResponse, ContractError>
where
    C: Clock,
    F: FnMut(ScanProgressEvent) + Send + 'static,
{
    let prepared = {
        let settings_store = state.settings.lock().map_err(|_| lock_state_error())?;
        let settings = settings_store.core();
        let settings_gen = settings_store.generation();
        let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
        sessions.begin_scan_checked(
            &state.clock,
            crate::session::scan::ScanBeginRequest {
                tiers,
                settings,
                settings_generation: settings_gen,
            },
        )?
    };
    let session_id = prepared.session_id.clone();
    let shared_cb = on_progress.map(|cb| Arc::new(Mutex::new(cb)));
    emit_scan_event(
        &shared_cb,
        ScanProgressEvent::Started {
            session_id: session_id.clone(),
        },
    );
    let core_cb = build_scan_progress_cb(&session_id, &shared_cb);
    let scan_result = scanner::scan_with_settings_options_current_elevation_with_progress(
        &prepared.effective_settings,
        scanner::ScanSettingsOptions::default(),
        scanner::ScanRunControl {
            cancel: prepared.cancel.clone(),
            progress_cb: core_cb,
        },
    );
    let ctx = ScanRunCtx {
        session_id: &session_id,
        prepared: &prepared,
        shared_cb: &shared_cb,
    };
    finalize_scan(state, ctx, scan_result)
}

fn finalize_scan<C, F>(
    state: &AppState<C>,
    ctx: ScanRunCtx<'_, F>,
    scan_result: Result<scanner::ScanReport, ScanError>,
) -> Result<StartScanResponse, ContractError>
where
    C: Clock,
    F: FnMut(ScanProgressEvent),
{
    match scan_result {
        Ok(report) => {
            // Build the small log record from a borrow of the report so the
            // (potentially very large) report is never deep-cloned (#96). The
            // record is written after a successful commit, as before.
            let log_record =
                build_scan_activity_log_record(&ctx.prepared.effective_settings, &report);
            if let Err(err) = commit_scan_outcome(state, ctx.session_id, ctx.prepared, report) {
                return Err(handle_scan_terminal(state, ctx, ScanTerminal::Failed(err)));
            }
            if let Some(record) = log_record {
                write_scan_log_record(record);
            }
            emit_scan_event(
                ctx.shared_cb,
                ScanProgressEvent::Completed {
                    session_id: ctx.session_id.to_string(),
                },
            );
            Ok(StartScanResponse {
                session_id: ctx.session_id.to_string(),
            })
        }
        Err(ScanError::Cancelled) => Err(handle_scan_terminal(state, ctx, ScanTerminal::Cancelled)),
        Err(e) => Err(handle_scan_terminal(
            state,
            ctx,
            ScanTerminal::Failed(ContractError::Internal {
                reason: crate::session::sanitize_reason_pub(crate::session::MsgRef(&e.to_string())),
            }),
        )),
    }
}

/// `cancel_scan` implementation.
///
/// Flags only the named in-flight `ScanSession`. A stale or unknown id is a
/// safe no-op; no session store entry, preview token, or unrelated session is
/// touched (#94).
pub fn cancel_scan<C: Clock>(state: &AppState<C>, session_id: String) -> Result<(), ContractError> {
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.cancel_scan(&session_id);
    Ok(())
}

/// `get_scan_results` implementation.
pub fn get_scan_results<C: Clock>(
    state: &AppState<C>,
    session_id: String,
) -> Result<ScanResultsDto, ContractError> {
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.get_scan_results(&state.clock, &session_id, settings_gen)
}

/// `preview_clean` implementation.
pub fn preview_clean<C: Clock>(
    state: &AppState<C>,
    session_id: String,
    item_ids: Vec<String>,
    confirmations: Option<PreviewConfirmationsDto>,
) -> Result<PreviewCleanResponse, ContractError> {
    let item_ids = crate::session::compact_selection::expand_selection_ids(&item_ids);
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.preview_clean(
        &state.clock,
        crate::session::PreviewCleanRequest {
            session_id,
            item_ids,
            confirmations,
            current_settings_generation: settings_gen,
        },
    )
}

/// `explain_item` implementation.
pub fn explain_item<C: Clock>(
    state: &AppState<C>,
    item_id: String,
) -> Result<ExplainItemResponse, ContractError> {
    let settings = state.settings.lock().map_err(|_| lock_state_error())?;
    let settings_gen = settings.generation();
    let mut sessions = state.sessions.lock().map_err(|_| lock_state_error())?;
    sessions.explain_item(&state.clock, &item_id, settings_gen)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use sweeper_core::scanner::{ScanItem, TargetScanResult};
    use sweeper_core::settings::SweeperSettings;
    use sweeper_core::{Risk, Tier};
    use sweeper_testkit::Sandbox;

    // All paths below come from a per-test Sandbox temp root. They are synthetic
    // fixtures resolved at runtime — never a real user name or machine path.
    fn log_test_report(root: &Path) -> scanner::ScanReport {
        let items = vec![ScanItem {
            id: "user-temp::a".to_string(),
            target_id: "user-temp".to_string(),
            path: root.join("a.tmp").to_string_lossy().into_owned(),
            file_identity: None,
            size: 10,
            mtime_unix_secs: 1_600_000_000,
            age_eligible: true,
        }];
        scanner::ScanReport {
            started_unix_secs: 1_800_000_000,
            min_age_hours: 24,
            targets: vec![
                TargetScanResult {
                    target_id: "user-temp".to_string(),
                    target_name: "User Temp".to_string(),
                    tier: Tier::Tier1,
                    risk: Risk::Safe,
                    resolved_paths: vec![root.join("Temp").to_string_lossy().into_owned()],
                    scanned: true,
                    status_reason: None,
                    total_bytes: 10,
                    eligible_bytes: 10,
                    file_count: 1,
                    eligible_count: 1,
                    skipped_count: 2,
                    skipped_reasons: Default::default(),
                    skipped_diagnostics: Vec::new(),
                    items,
                },
                TargetScanResult {
                    target_id: "empty-target".to_string(),
                    target_name: "Empty".to_string(),
                    tier: Tier::Tier2,
                    risk: Risk::Caution,
                    resolved_paths: vec![root.join("Empty").to_string_lossy().into_owned()],
                    scanned: true,
                    status_reason: None,
                    total_bytes: 0,
                    eligible_bytes: 0,
                    file_count: 0,
                    eligible_count: 0,
                    skipped_count: 0,
                    skipped_reasons: Default::default(),
                    skipped_diagnostics: Vec::new(),
                    items: Vec::new(),
                },
            ],
        }
    }

    /// Memory oracle (#96): prove the log-record builder borrows the report
    /// instead of copying it. A deep clone would allocate fresh `String`s for
    /// the target metadata, so the borrowed view's string references would no
    /// longer be the very same slices as the source report's — this test fails
    /// in that case. Because the view holds only references and primitives, the
    /// builder has no owned report data to copy, which keeps the logging path
    /// O(targets) and independent of the item count.
    #[test]
    fn log_record_build_borrows_report_fields_without_copying() {
        let sandbox = Sandbox::new();
        let report = log_test_report(sandbox.root());
        let settings = SweeperSettings {
            enable_tier1: true,
            ..SweeperSettings::default()
        };

        // Drive the real production builder; the observer runs with the exact
        // borrowed view the record is assembled from, so a deep copy inserted
        // anywhere in the record-building path fails the identity checks below.
        let record = build_scan_activity_log_record_observed(&settings, &report, |borrowed| {
            assert_eq!(borrowed.len(), report.targets.len());
            for (view, source) in borrowed.iter().zip(report.targets.iter()) {
                // `std::ptr::eq` compares the data pointer AND length of `&str`.
                // Identical slices prove no per-target `String` was copied or
                // cloned — the view aliases the live report, not a copy.
                assert!(
                    std::ptr::eq(view.target_id, source.target_id.as_str()),
                    "target_id was copied instead of borrowed (deep clone reintroduced)"
                );
                assert!(
                    std::ptr::eq(view.target_name, source.target_name.as_str()),
                    "target_name was copied instead of borrowed (deep clone reintroduced)"
                );
                assert_eq!(view.eligible_bytes, source.eligible_bytes);
                assert_eq!(view.file_count, source.file_count);
            }
        })
        .expect("non-sandbox scan logs");

        assert_eq!(record["categories_count"], 1);
        // Even after the record is built, the original report is untouched and
        // fully usable: it was never moved or deep-cloned.
        assert_eq!(report.targets[0].items[0].id, "user-temp::a");
    }

    #[test]
    fn scan_log_record_saturates_overflowing_totals() {
        let sandbox = Sandbox::new();
        let settings = SweeperSettings {
            enable_tier1: true,
            ..SweeperSettings::default()
        };
        let mut report = log_test_report(sandbox.root());
        report.targets[0].eligible_bytes = u64::MAX;
        report.targets[0].eligible_count = u64::MAX;
        report.targets[0].skipped_count = u64::MAX;
        // A second non-zero target makes a reverted plain `.sum()` overflow
        // (and fail) instead of silently producing the same u64::MAX total.
        report.targets[1].eligible_bytes = 1;
        report.targets[1].eligible_count = 1;
        report.targets[1].skipped_count = 1;

        let record =
            build_scan_activity_log_record(&settings, &report).expect("non-sandbox scan logs");
        assert_eq!(record["total_reclaimable_bytes"], u64::MAX.to_string());
        assert_eq!(record["total_eligible_files"], u64::MAX.to_string());
        assert_eq!(record["total_skipped_files"], u64::MAX.to_string());
    }

    #[test]
    fn log_record_is_built_from_a_borrow_and_preserves_totals() {
        let sandbox = Sandbox::new();
        let settings = SweeperSettings {
            enable_tier1: true,
            ..SweeperSettings::default()
        };
        let report = log_test_report(sandbox.root());

        let record =
            build_scan_activity_log_record(&settings, &report).expect("non-sandbox scan logs");

        assert_eq!(record["kind"], "scan");
        assert_eq!(record["total_reclaimable_bytes"], "10");
        assert_eq!(record["total_eligible_files"], "1");
        assert_eq!(record["total_skipped_files"], "2");
        assert_eq!(record["categories_count"], 1);
        assert_eq!(record["targets"][0]["target_id"], "user-temp");

        // The report was borrowed, never moved or deep-cloned, and remains
        // entirely usable here.
        assert_eq!(report.targets.len(), 2);
        assert_eq!(report.targets[0].items[0].id, "user-temp::a");
    }

    #[test]
    fn sandbox_scan_produces_no_log_record() {
        let sandbox = Sandbox::new();
        // A synthetic path containing the sentinel marker component is enough
        // for the sandbox hint; it never touches a real user or system path.
        let sentinel = sandbox
            .guarded_path(".sweeper_sandbox_sentinel/cache")
            .to_string_lossy()
            .into_owned();
        let settings = SweeperSettings {
            enable_tier1: false,
            enable_tier2: false,
            enable_tier3: false,
            custom_paths: vec![sentinel.clone()],
            ..SweeperSettings::default()
        };
        let mut report = log_test_report(sandbox.root());
        report.targets = report
            .targets
            .into_iter()
            .map(|mut t| {
                t.target_id = "custom-path-sandbox".to_string();
                t.resolved_paths = vec![sentinel.clone()];
                t
            })
            .collect();

        assert!(build_scan_activity_log_record(&settings, &report).is_none());
    }
}
