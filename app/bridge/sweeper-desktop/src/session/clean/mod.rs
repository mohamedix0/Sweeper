use std::collections::BTreeMap;
use std::path::PathBuf;

use sweeper_contract::{PreviewConfirmationsDto, PreviewTargetBreakdownDto};
use sweeper_core::remover::ValidatedItem;

use super::SessionStore;
use crate::clock::Clock;
use sweeper_contract::ContractError;

mod execute;
mod explain;
mod preview;

/// Cohesive request for previewing a clean selection.
#[derive(Debug)]
pub struct PreviewCleanRequest {
    /// Source scan/analyzer session id.
    pub session_id: String,
    /// Selected item IDs.
    pub item_ids: Vec<String>,
    /// Optional typed confirmations.
    pub confirmations: Option<PreviewConfirmationsDto>,
    /// Current settings generation for session validation.
    pub current_settings_generation: u64,
}

/// Cohesive request for executing a preview token.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ExecuteCleanRequest {
    /// Preview token to consume and execute.
    pub preview_token: String,
    /// Current settings generation for validation.
    pub current_settings_generation: u64,
    /// Optional journal/log base directory.
    pub journal_base_dir: Option<PathBuf>,
}

#[derive(Debug)]
pub(super) struct PreviewRequest {
    pub(super) session_id: String,
    pub(super) item_ids: Vec<String>,
    pub(super) confirmations: Option<PreviewConfirmationsDto>,
    pub(super) settings_generation: u64,
}

#[derive(Debug)]
pub(super) struct ScanPreviewBuild {
    pub(super) total_bytes: u64,
    pub(super) validated_items: Vec<ValidatedItem>,
    pub(super) breakdown: Vec<PreviewTargetBreakdownDto>,
}

#[derive(Debug)]
pub(super) struct AnalyzerPreviewBuild {
    pub(super) total_bytes: u64,
    pub(super) validated_items: Vec<ValidatedItem>,
    pub(super) breakdown: Vec<PreviewTargetBreakdownDto>,
}

pub(super) struct ScanAssembly<'a> {
    pub(super) by_target: &'a mut BTreeMap<String, (String, u64, u64)>,
    pub(super) total_bytes: &'a mut u64,
    pub(super) validated: &'a mut Vec<ValidatedItem>,
}

impl SessionStore {
    pub(super) fn get_valid_scan_session<C: Clock>(
        &self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<&super::ScanSession, ContractError> {
        self.get_valid_session(clock, session_id, current_settings_generation, |s| {
            (&s.sessions, super::mapping::scan_session_valid_at)
        })
    }

    pub(super) fn get_valid_analyzer_session<C: Clock>(
        &self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<&super::AnalyzerSession, ContractError> {
        self.get_valid_session(clock, session_id, current_settings_generation, |s| {
            (
                &s.analyzer_sessions,
                super::mapping::analyzer_session_valid_at,
            )
        })
    }

    fn get_valid_session<'a, C, T, M, V>(
        &'a self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
        map: M,
    ) -> Result<&'a T, ContractError>
    where
        C: Clock,
        M: FnOnce(&'a Self) -> (&'a std::collections::HashMap<String, T>, V),
        V: Fn(&T, u64, u64) -> bool,
    {
        let now = clock.now_unix_secs();
        let (store, valid_at) = map(self);
        let session = store
            .get(session_id)
            .ok_or_else(|| ContractError::SessionInvalid {
                reason: "session not found".to_string(),
            })?;
        if valid_at(session, now, current_settings_generation) {
            Ok(session)
        } else {
            Err(ContractError::SessionInvalid {
                reason: "session expired or invalidated".to_string(),
            })
        }
    }

    pub(super) fn get_valid_session_meta<C: Clock>(
        &self,
        clock: &C,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Result<(u64, u32), ContractError> {
        let now = clock.now_unix_secs();
        self.scan_session_meta(now, session_id, current_settings_generation)
            .or_else(|| self.analyzer_session_meta(now, session_id, current_settings_generation))
            .ok_or_else(|| ContractError::SessionInvalid {
                reason: "session not found".to_string(),
            })?
    }

    fn scan_session_meta(
        &self,
        now: u64,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Option<Result<(u64, u32), ContractError>> {
        let s = self.sessions.get(session_id)?;
        Some(Self::meta_if_valid(
            super::mapping::scan_session_valid_at(s, now, current_settings_generation),
            s.created_at,
            s.min_age_hours,
        ))
    }

    fn analyzer_session_meta(
        &self,
        now: u64,
        session_id: &str,
        current_settings_generation: u64,
    ) -> Option<Result<(u64, u32), ContractError>> {
        let s = self.analyzer_sessions.get(session_id)?;
        Some(Self::meta_if_valid(
            super::mapping::analyzer_session_valid_at(s, now, current_settings_generation),
            s.created_at,
            s.min_age_hours,
        ))
    }

    fn meta_if_valid(
        valid: bool,
        created_at: u64,
        min_age: u32,
    ) -> Result<(u64, u32), ContractError> {
        if valid {
            Ok((created_at, min_age))
        } else {
            Err(ContractError::SessionInvalid {
                reason: "session expired or invalidated".to_string(),
            })
        }
    }
}
