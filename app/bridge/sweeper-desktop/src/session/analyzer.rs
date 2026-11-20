use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sweeper_contract::{AnalyzerProgressEvent, ContractError, StartAnalyzerResponse, U64String};
use sweeper_core::analyzer::{self, AnalyzerOptions, AnalyzerProgress};
use sweeper_core::settings::SweeperSettings;

use super::mapping::{map_analyzer_error, map_analyzer_result_to_dto};
use super::{AnalyzerFinishInput, AnalyzerSessionInput, PreparedAnalyzer, SessionStore};
use crate::clock::Clock;

/// Cohesive request for synchronous analyzer store helper.
#[derive(Debug, Clone)]
pub struct StoreAnalyzerRun {
    /// Optional root path.
    pub root: Option<String>,
    /// Settings snapshot.
    pub settings: SweeperSettings,
    /// Scan-affecting settings generation for this run.
    pub settings_generation: u64,
    /// Analyzer-affecting settings generation for this run.
    pub analyzer_generation: u64,
}

impl SessionStore {
    /// Register and prepare an analyzer run (short lock).
    pub fn begin_analyzer<C: Clock>(
        &mut self,
        clock: &C,
        settings_generation: u64,
        analyzer_generation: u64,
    ) -> Result<PreparedAnalyzer, ContractError> {
        self.cleanup(clock);
        self.cancel_all_active_analyzers();
        let now = clock.now_unix_secs();
        let session_id = self.next_id("analyzer", now);
        let cancel = Arc::new(AtomicBool::new(false));
        self.active_analyzers
            .insert(session_id.clone(), cancel.clone());
        Ok(PreparedAnalyzer {
            session_id,
            settings_generation,
            analyzer_generation,
            cancel,
        })
    }

    /// Request cancellation for an active analyzer session.
    pub fn cancel_analyzer(&mut self, session_id: &str) -> bool {
        if let Some(flag) = self.active_analyzers.get(session_id) {
            flag.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// Cancel all active analyzer runs (used when starting a new run).
    pub fn cancel_all_active_analyzers(&mut self) {
        for flag in self.active_analyzers.values() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Commit a completed analyzer session (short lock).
    pub fn finish_analyzer<C: Clock>(
        &mut self,
        clock: &C,
        input: AnalyzerFinishInput,
    ) -> Result<StartAnalyzerResponse, ContractError> {
        self.active_analyzers.remove(&input.session_id);
        if input.settings_generation_at_start != input.current_settings_generation
            || input.analyzer_generation_at_start != input.current_analyzer_generation
        {
            return Err(ContractError::SessionInvalid {
                reason: "session invalidated by settings change".to_string(),
            });
        }
        let now = clock.now_unix_secs();
        let dto = map_analyzer_result_to_dto(&input.session_id, &input.result);
        self.insert_analyzer_session(AnalyzerSessionInput {
            id: input.session_id.clone(),
            created_at: now,
            settings_generation: input.settings_generation_at_start,
            result: input.result,
            min_age_hours: input.min_age_hours,
        });
        Ok(dto)
    }

    /// Mark an analyzer run as no longer active.
    pub fn abort_analyzer(&mut self, session_id: &str) {
        self.active_analyzers.remove(session_id);
    }

    /// Start a read-only analyzer session and store it (synchronous helper).
    pub fn start_analyzer<C, P>(
        &mut self,
        clock: &C,
        run: StoreAnalyzerRun,
        progress_cb: Option<P>,
    ) -> Result<StartAnalyzerResponse, ContractError>
    where
        C: Clock,
        P: Fn(AnalyzerProgressEvent) + Send + Sync + 'static,
    {
        let prepared =
            self.begin_analyzer(clock, run.settings_generation, run.analyzer_generation)?;
        let progress = Self::analyzer_progress_cb(&prepared.session_id, progress_cb);
        let options = AnalyzerOptions {
            allow_unknown: run.settings.allow_delete_unknown,
            ..AnalyzerOptions::default()
        };
        let result = match analyzer::analyze_path(
            run.root.as_deref(),
            prepared.cancel.clone(),
            progress,
            options,
        ) {
            Ok(r) => r,
            Err(e) => {
                self.abort_analyzer(&prepared.session_id);
                return Err(map_analyzer_error(e));
            }
        };
        self.finish_analyzer(
            clock,
            AnalyzerFinishInput {
                session_id: prepared.session_id,
                settings_generation_at_start: prepared.settings_generation,
                current_settings_generation: run.settings_generation,
                analyzer_generation_at_start: prepared.analyzer_generation,
                current_analyzer_generation: run.analyzer_generation,
                min_age_hours: run.settings.min_age_hours,
                result,
            },
        )
    }

    fn analyzer_progress_cb<P>(
        session_id: &str,
        progress_cb: Option<P>,
    ) -> Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>
    where
        P: Fn(AnalyzerProgressEvent) + Send + Sync + 'static,
    {
        let cb = progress_cb?;
        let sid = session_id.to_string();
        Some(Arc::new(move |p: AnalyzerProgress| {
            cb(AnalyzerProgressEvent {
                session_id: sid.clone(),
                scanned_dirs: U64String(p.scanned_dirs),
                scanned_files: U64String(p.scanned_files),
                scanned_bytes: U64String(p.scanned_bytes),
                current_path: p.current_path,
            });
        }))
    }
}
