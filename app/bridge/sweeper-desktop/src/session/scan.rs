use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use sweeper_contract::{
    ContractError, ScanProgressEvent, ScanResultsDto, StartScanResponse, TierDto,
};
use sweeper_core::scanner::{self, ScanError, ScanReport};
use sweeper_core::settings::SweeperSettings;

use super::mapping::map_report_to_session;
use super::support::{build_store_scan_cb, emit_scan_progress, sanitize_reason, SessionIdRef};
use super::{PreparedScan, ScanFinishInput, ScanSession, SessionStore};
use crate::clock::Clock;

type SharedScanCb<F> = Arc<Mutex<F>>;

#[derive(Clone)]
struct StoreScanCtx {
    session_id: String,
    prepared: PreparedScan,
    current_settings_generation: u64,
}

struct StoreScanFinalizeInput {
    ctx: StoreScanCtx,
    scan_result: Result<ScanReport, ScanError>,
}

struct StoreScanCompleteInput {
    ctx: StoreScanCtx,
    report: ScanReport,
}

/// Cohesive request for running a blocking scan with progress in store helpers.
#[derive(Debug, Clone)]
pub struct ScanRunRequest {
    /// Optional tier filter.
    pub tiers: Option<Vec<TierDto>>,
    /// Base settings snapshot.
    pub settings: SweeperSettings,
    /// Settings generation at scan start.
    pub settings_generation_at_start: u64,
    /// Current settings generation for commit check.
    pub current_settings_generation: u64,
}

/// Cohesive request for beginning/preparing a scan run.
#[derive(Debug, Clone)]
pub struct ScanBeginRequest {
    /// Optional tier filter.
    pub tiers: Option<Vec<TierDto>>,
    /// Base settings snapshot.
    pub settings: SweeperSettings,
    /// Settings generation at scan start.
    pub settings_generation: u64,
}

impl SessionStore {
    /// Register and prepare a scan run (short lock).
    pub fn begin_scan<C: Clock>(
        &mut self,
        clock: &C,
        req: ScanBeginRequest,
    ) -> Result<PreparedScan, ContractError> {
        self.cleanup(clock);
        let now = clock.now_unix_secs();
        let effective = Self::effective_scan_settings(req.tiers, &req.settings);
        let session_id = self.next_id("scan", now);
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_scans.insert(session_id.clone(), cancel.clone());
        Ok(PreparedScan {
            session_id,
            effective_settings: effective,
            settings_generation: req.settings_generation,
            cancel,
        })
    }

    fn effective_scan_settings(
        tiers: Option<Vec<TierDto>>,
        settings: &SweeperSettings,
    ) -> SweeperSettings {
        let mut effective = settings.clone();
        let Some(req) = tiers.as_deref().filter(|t| !t.is_empty()) else {
            return effective;
        };
        let req_set: HashSet<TierDto> = req.iter().copied().collect();
        effective.enable_tier1 = effective.enable_tier1 && req_set.contains(&TierDto::Tier1);
        effective.enable_tier2 = effective.enable_tier2 && req_set.contains(&TierDto::Tier2);
        effective.enable_tier3 = effective.enable_tier3 && req_set.contains(&TierDto::Tier3);
        effective
    }

    /// Commit a completed scan session (short lock).
    ///
    /// Always clears active scan tracking for `session_id`, even when commit is rejected.
    pub fn finish_scan<C: Clock>(
        &mut self,
        clock: &C,
        input: ScanFinishInput,
    ) -> Result<(), ContractError> {
        self.active_scans.remove(&input.session_id);
        if input.settings_generation_at_start != input.current_settings_generation {
            return Err(ContractError::SessionInvalid {
                reason: "session invalidated by settings change".to_string(),
            });
        }
        self.insert_finished_scan(clock, input);
        Ok(())
    }

    fn insert_finished_scan<C: Clock>(&mut self, clock: &C, input: ScanFinishInput) {
        let now = clock.now_unix_secs();
        let (results, items, target_roots) = map_report_to_session(&input.session_id, input.report);
        let session = ScanSession {
            id: input.session_id.clone(),
            created_at: now,
            settings_generation: input.settings_generation_at_start,
            results,
            items,
            target_roots,
            min_age_hours: input.effective_min_age_hours,
        };
        self.sessions.insert(input.session_id, session);
    }

    /// Mark a scan as no longer active (failure/cancel).
    pub fn abort_scan(&mut self, session_id: &str) {
        self.active_scans.remove(session_id);
    }

    /// Create and store a scan session by running a read-only scan (synchronous helper).
    pub fn start_scan<C: Clock>(
        &mut self,
        clock: &C,
        req: ScanRunRequest,
    ) -> Result<StartScanResponse, ContractError> {
        self.start_scan_with_progress(clock, req, None::<fn(ScanProgressEvent)>)
    }

    /// Create and store a scan session with optional progress callback (synchronous helper).
    pub fn start_scan_with_progress<C, F>(
        &mut self,
        clock: &C,
        req: ScanRunRequest,
        progress_cb: Option<F>,
    ) -> Result<StartScanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(ScanProgressEvent) + Send + 'static,
    {
        let prepared = self.begin_scan(
            clock,
            ScanBeginRequest {
                tiers: req.tiers,
                settings: req.settings.clone(),
                settings_generation: req.settings_generation_at_start,
            },
        )?;
        let ctx = StoreScanCtx {
            session_id: prepared.session_id.clone(),
            prepared,
            current_settings_generation: req.current_settings_generation,
        };
        self.run_prepared_scan(clock, ctx, progress_cb)
    }

    fn run_prepared_scan<C, F>(
        &mut self,
        clock: &C,
        ctx: StoreScanCtx,
        progress_cb: Option<F>,
    ) -> Result<StartScanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(ScanProgressEvent) + Send + 'static,
    {
        let shared_cb = progress_cb.map(|cb| Arc::new(Mutex::new(cb)));
        emit_scan_progress(
            &shared_cb,
            ScanProgressEvent::Started {
                session_id: ctx.session_id.clone(),
            },
        );
        let core_cb = build_store_scan_cb(SessionIdRef(&ctx.session_id), &shared_cb);
        let scan_result = scanner::scan_with_settings_current_elevation_with_progress(
            &ctx.prepared.effective_settings,
            scanner::ScanRunControl {
                cancel: ctx.prepared.cancel.clone(),
                progress_cb: core_cb,
            },
        );
        self.finalize_store_scan(
            clock,
            &shared_cb,
            StoreScanFinalizeInput { ctx, scan_result },
        )
    }

    fn finalize_store_scan<C, F>(
        &mut self,
        clock: &C,
        shared_cb: &Option<SharedScanCb<F>>,
        input: StoreScanFinalizeInput,
    ) -> Result<StartScanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(ScanProgressEvent),
    {
        match input.scan_result {
            Ok(report) => self.complete_store_scan(
                clock,
                shared_cb,
                StoreScanCompleteInput {
                    ctx: input.ctx,
                    report,
                },
            ),
            Err(ScanError::Cancelled) => {
                Err(self.fail_store_scan_cancelled(&input.ctx.session_id, shared_cb))
            }
            Err(e) => Err(self.fail_store_scan_error(&input.ctx.session_id, shared_cb, e)),
        }
    }

    fn complete_store_scan<C, F>(
        &mut self,
        clock: &C,
        shared_cb: &Option<SharedScanCb<F>>,
        input: StoreScanCompleteInput,
    ) -> Result<StartScanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(ScanProgressEvent),
    {
        let ctx = input.ctx;
        let finish = ScanFinishInput {
            session_id: ctx.session_id.clone(),
            settings_generation_at_start: ctx.prepared.settings_generation,
            current_settings_generation: ctx.current_settings_generation,
            effective_min_age_hours: ctx.prepared.effective_settings.min_age_hours,
            report: input.report,
        };
        if let Err(err) = self.finish_scan(clock, finish) {
            emit_scan_progress(
                shared_cb,
                ScanProgressEvent::Failed {
                    session_id: ctx.session_id,
                    error: err.clone(),
                },
            );
            return Err(err);
        }
        emit_scan_progress(
            shared_cb,
            ScanProgressEvent::Completed {
                session_id: ctx.session_id.clone(),
            },
        );
        Ok(StartScanResponse {
            session_id: ctx.session_id,
        })
    }

    fn fail_store_scan_cancelled<F>(
        &mut self,
        session_id: &str,
        shared_cb: &Option<SharedScanCb<F>>,
    ) -> ContractError
    where
        F: FnMut(ScanProgressEvent),
    {
        self.abort_scan(session_id);
        emit_scan_progress(
            shared_cb,
            ScanProgressEvent::Cancelled {
                session_id: session_id.to_string(),
            },
        );
        ContractError::Internal {
            reason: "scan cancelled".to_string(),
        }
    }

    fn fail_store_scan_error<F>(
        &mut self,
        session_id: &str,
        shared_cb: &Option<SharedScanCb<F>>,
        e: ScanError,
    ) -> ContractError
    where
        F: FnMut(ScanProgressEvent),
    {
        self.abort_scan(session_id);
        let mapped = ContractError::Internal {
            reason: sanitize_reason(&e.to_string()),
        };
        emit_scan_progress(
            shared_cb,
            ScanProgressEvent::Failed {
                session_id: session_id.to_string(),
                error: mapped.clone(),
            },
        );
        mapped
    }

    /// Get scan results for a valid scan session.
    pub fn get_scan_results<C: Clock>(
        &mut self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<ScanResultsDto, ContractError> {
        self.cleanup(clock);
        let lookup = super::clean::SessionLookup::new(session_id, current_settings_generation);
        let session = self.get_valid_scan_session(clock, lookup)?;
        Ok(session.results.clone())
    }
}
