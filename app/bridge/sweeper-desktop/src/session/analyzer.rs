use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sweeper_contract::{ContractError, StartAnalyzerResponse};

use super::{AnalyzerFinishInput, AnalyzerSessionInput, PreparedAnalyzer, SessionStore};
use crate::clock::Clock;

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
    ///
    /// On a generation mismatch the whole input is handed back with the error so
    /// the caller can drop the (potentially large) response DTO tree *after* it
    /// has released the commit guards, instead of extending the locked section
    /// with a large-tree drop (#96).
    pub fn finish_analyzer<C: Clock>(
        &mut self,
        clock: &C,
        input: AnalyzerFinishInput,
    ) -> Result<StartAnalyzerResponse, (ContractError, Box<AnalyzerFinishInput>)> {
        self.active_analyzers.remove(&input.session_id);
        if input.settings_generation_at_start != input.current_settings_generation
            || input.analyzer_generation_at_start != input.current_analyzer_generation
        {
            return Err((
                ContractError::SessionInvalid {
                    reason: "session invalidated by settings change".to_string(),
                },
                Box::new(input),
            ));
        }
        let now = clock.now_unix_secs();
        self.insert_analyzer_session(AnalyzerSessionInput {
            id: input.session_id.clone(),
            created_at: now,
            settings_generation: input.settings_generation_at_start,
            result: input.result,
            min_age_hours: input.min_age_hours,
        });
        Ok(input.dto)
    }

    /// Mark an analyzer run as no longer active.
    pub fn abort_analyzer(&mut self, session_id: &str) {
        self.active_analyzers.remove(session_id);
    }
}
