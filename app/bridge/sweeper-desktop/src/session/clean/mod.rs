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

#[derive(Clone, Copy, Debug)]
pub(super) struct SessionLookup<'a> {
    pub session_id: &'a str,
    pub current_settings_generation: u64,
}

impl<'a> SessionLookup<'a> {
    pub fn new(session_id: &'a str, current_settings_generation: u64) -> Self {
        Self {
            session_id,
            current_settings_generation,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum SessionKind {
    Scan,
    Analyzer,
}

impl SessionStore {
    pub(super) fn scan_sessions(&self) -> &std::collections::HashMap<String, super::ScanSession> {
        &self.sessions
    }

    pub(super) fn analyzer_sessions(
        &self,
    ) -> &std::collections::HashMap<String, super::AnalyzerSession> {
        &self.analyzer_sessions
    }

    pub(super) fn get_valid_session<C, T, M>(
        &self,
        clock: &C,
        lookup: SessionLookup<'_>,
        map: M,
    ) -> Result<&T, ContractError>
    where
        C: Clock,
        T: SessionMeta,
        M: FnOnce(&Self) -> &std::collections::HashMap<String, T>,
    {
        let now = clock.now_unix_secs();
        let session =
            map(self)
                .get(lookup.session_id)
                .ok_or_else(|| ContractError::SessionInvalid {
                    reason: "session not found".to_string(),
                })?;
        let valid = super::mapping::session_fields_valid_at(
            session.created_at(),
            session.settings_generation(),
            now,
            lookup.current_settings_generation,
        );
        if valid {
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
        lookup: SessionLookup<'_>,
    ) -> Result<(u64, u32), ContractError> {
        let now = clock.now_unix_secs();
        self.session_meta(now, lookup, SessionKind::Scan)
            .or_else(|| self.session_meta(now, lookup, SessionKind::Analyzer))
            .ok_or_else(|| ContractError::SessionInvalid {
                reason: "session not found".to_string(),
            })?
    }

    fn session_meta(
        &self,
        now: u64,
        lookup: SessionLookup<'_>,
        kind: SessionKind,
    ) -> Option<Result<(u64, u32), ContractError>> {
        match kind {
            SessionKind::Scan => self.meta_from(self.scan_sessions(), now, lookup),
            SessionKind::Analyzer => self.meta_from(self.analyzer_sessions(), now, lookup),
        }
    }

    fn meta_from<T: SessionMeta>(
        &self,
        map: &std::collections::HashMap<String, T>,
        now: u64,
        lookup: SessionLookup<'_>,
    ) -> Option<Result<(u64, u32), ContractError>> {
        let s = map.get(lookup.session_id)?;
        let valid = super::mapping::session_fields_valid_at(
            s.created_at(),
            s.settings_generation(),
            now,
            lookup.current_settings_generation,
        );
        Some(Self::meta_if_valid(
            valid,
            s.created_at(),
            s.min_age_hours(),
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

pub(super) trait SessionMeta {
    fn created_at(&self) -> u64;
    fn min_age_hours(&self) -> u32;
    fn settings_generation(&self) -> u64;
}

impl SessionMeta for super::ScanSession {
    fn created_at(&self) -> u64 {
        self.created_at
    }
    fn min_age_hours(&self) -> u32 {
        self.min_age_hours
    }
    fn settings_generation(&self) -> u64 {
        self.settings_generation
    }
}

impl SessionMeta for super::AnalyzerSession {
    fn created_at(&self) -> u64 {
        self.created_at
    }
    fn min_age_hours(&self) -> u32 {
        self.min_age_hours
    }
    fn settings_generation(&self) -> u64 {
        self.settings_generation
    }
}
