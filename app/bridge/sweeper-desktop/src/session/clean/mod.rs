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

struct SessionAccess<'a, C, M, V> {
    clock: &'a C,
    lookup: SessionLookup<'a>,
    map: M,
    valid_at: V,
}

struct SessionMetaSpec<'a, M, V> {
    now: u64,
    lookup: SessionLookup<'a>,
    map: M,
    valid_at: V,
}

impl SessionStore {
    pub(super) fn get_valid_scan_session<C: Clock>(
        &self,
        clock: &C,
        lookup: SessionLookup<'_>,
    ) -> Result<&super::ScanSession, ContractError> {
        self.get_valid_session(SessionAccess {
            clock,
            lookup,
            map: Self::scan_sessions,
            valid_at: super::mapping::scan_session_valid_at,
        })
    }

    pub(super) fn get_valid_analyzer_session<C: Clock>(
        &self,
        clock: &C,
        lookup: SessionLookup<'_>,
    ) -> Result<&super::AnalyzerSession, ContractError> {
        self.get_valid_session(SessionAccess {
            clock,
            lookup,
            map: Self::analyzer_sessions_map,
            valid_at: super::mapping::analyzer_session_valid_at,
        })
    }

    fn scan_sessions(&self) -> &std::collections::HashMap<String, super::ScanSession> {
        &self.sessions
    }

    fn analyzer_sessions_map(&self) -> &std::collections::HashMap<String, super::AnalyzerSession> {
        &self.analyzer_sessions
    }

    fn get_valid_session<'a, C, T, M, V>(
        &'a self,
        access: SessionAccess<'_, C, M, V>,
    ) -> Result<&'a T, ContractError>
    where
        C: Clock,
        T: 'a,
        M: FnOnce(&'a Self) -> &'a std::collections::HashMap<String, T>,
        V: Fn(&T, u64, u64) -> bool,
    {
        let now = access.clock.now_unix_secs();
        let session = (access.map)(self)
            .get(access.lookup.session_id)
            .ok_or_else(|| ContractError::SessionInvalid {
                reason: "session not found".to_string(),
            })?;
        if (access.valid_at)(session, now, access.lookup.current_settings_generation) {
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
        self.session_meta(SessionMetaSpec {
            now,
            lookup,
            map: Self::scan_sessions,
            valid_at: super::mapping::scan_session_valid_at,
        })
        .or_else(|| {
            self.session_meta(SessionMetaSpec {
                now,
                lookup,
                map: Self::analyzer_sessions_map,
                valid_at: super::mapping::analyzer_session_valid_at,
            })
        })
        .ok_or_else(|| ContractError::SessionInvalid {
            reason: "session not found".to_string(),
        })?
    }

    fn session_meta<'a, T, M, V>(
        &'a self,
        spec: SessionMetaSpec<'_, M, V>,
    ) -> Option<Result<(u64, u32), ContractError>>
    where
        T: SessionMeta + 'a,
        M: FnOnce(&'a Self) -> &'a std::collections::HashMap<String, T>,
        V: Fn(&T, u64, u64) -> bool,
    {
        let s = (spec.map)(self).get(spec.lookup.session_id)?;
        let valid = (spec.valid_at)(s, spec.now, spec.lookup.current_settings_generation);
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

trait SessionMeta {
    fn created_at(&self) -> u64;
    fn min_age_hours(&self) -> u32;
}

impl SessionMeta for super::ScanSession {
    fn created_at(&self) -> u64 {
        self.created_at
    }
    fn min_age_hours(&self) -> u32 {
        self.min_age_hours
    }
}

impl SessionMeta for super::AnalyzerSession {
    fn created_at(&self) -> u64 {
        self.created_at
    }
    fn min_age_hours(&self) -> u32 {
        self.min_age_hours
    }
}
