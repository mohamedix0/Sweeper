use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use sweeper_contract::{ContractError, ExecuteCleanResponse, SkippedReasonDto, U64String};
use sweeper_core::remover::{self, CleanOptions, CleanProgress, ValidatedItem};

use super::super::support::{
    detect_new_log_id, hash_selection, list_log_ids, sanitize_reason, SessionIdRef,
};
use super::super::{
    PreviewTokenState, SessionStore, ValidatedCleanBatch, PREVIEW_TOKEN_TTL_SECS,
    SCAN_SESSION_TTL_SECS,
};
use super::ExecuteCleanRequest;
use crate::clock::Clock;

impl SessionStore {
    /// Validate and atomically consume a preview token for execution.
    pub fn consume_preview_token_for_execute<C: Clock>(
        &mut self,
        clock: &C,
        preview_token: &str,
        current_settings_generation: u64,
    ) -> Result<(u32, Vec<ValidatedItem>, u64), ContractError> {
        self.cleanup(clock);
        self.reject_clean_while_scan_running()?;
        let now = clock.now_unix_secs();
        let snapshot = self.snapshot_preview_token(preview_token)?;
        Self::validate_preview_snapshot(now, &snapshot)?;
        let min_age =
            self.validate_preview_session(clock, &snapshot, current_settings_generation)?;
        self.mark_preview_used(preview_token)?;
        Ok((
            min_age,
            snapshot.validated_items,
            snapshot.preview_estimated_bytes,
        ))
    }

    fn snapshot_preview_token(
        &self,
        preview_token: &str,
    ) -> Result<PreviewTokenState, ContractError> {
        self.preview_tokens
            .get(preview_token)
            .cloned()
            .ok_or_else(|| ContractError::PreviewTokenInvalid {
                reason: "token not found".to_string(),
            })
    }

    fn validate_preview_snapshot(now: u64, t: &PreviewTokenState) -> Result<(), ContractError> {
        if t.used {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token already used".to_string(),
            });
        }
        if now.saturating_sub(t.issued_at) > PREVIEW_TOKEN_TTL_SECS {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token expired".to_string(),
            });
        }
        let expected = hash_selection(SessionIdRef(&t.session_id), &t.item_ids);
        if expected != t.selection_hash {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token tampered".to_string(),
            });
        }
        Ok(())
    }

    fn validate_preview_session<C: Clock>(
        &self,
        clock: &C,
        t: &PreviewTokenState,
        settings_gen: u64,
    ) -> Result<u32, ContractError> {
        let now = clock.now_unix_secs();
        let lookup = super::SessionLookup::new(&t.session_id, settings_gen);
        let (created_at, min_age) = self.get_valid_session_meta(clock, lookup)?;
        if now.saturating_sub(created_at) > SCAN_SESSION_TTL_SECS {
            return Err(ContractError::SessionInvalid {
                reason: "session expired".to_string(),
            });
        }
        Ok(min_age)
    }

    fn mark_preview_used(&mut self, preview_token: &str) -> Result<(), ContractError> {
        let t = self.preview_tokens.get_mut(preview_token).ok_or_else(|| {
            ContractError::PreviewTokenInvalid {
                reason: "token not found".to_string(),
            }
        })?;
        if t.used {
            return Err(ContractError::PreviewTokenInvalid {
                reason: "token already used".to_string(),
            });
        }
        t.used = true;
        Ok(())
    }

    /// Execute clean for a preview token (consumes token atomically; runs removal while holding `&mut self`).
    ///
    /// Prefer `commands_impl::execute_clean` for lock-split execution.
    pub fn execute_clean<C, F>(
        &mut self,
        clock: &C,
        req: ExecuteCleanRequest,
        on_progress: F,
    ) -> Result<ExecuteCleanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        let (min_age_hours, validated_items, preview_estimated_bytes) = self
            .consume_preview_token_for_execute(
                clock,
                &req.preview_token,
                req.current_settings_generation,
            )?;
        Self::execute_validated_clean_batch(
            clock,
            ValidatedCleanBatch {
                preview_token: req.preview_token,
                min_age_hours,
                validated_items,
                preview_estimated_bytes,
                journal_base_dir: req.journal_base_dir,
            },
            on_progress,
        )
    }

    /// Run clean removal for an already-validated, token-consumed batch (no SessionStore lock required).
    pub fn execute_validated_clean_batch<C, F>(
        clock: &C,
        batch: ValidatedCleanBatch,
        mut on_progress: F,
    ) -> Result<ExecuteCleanResponse, ContractError>
    where
        C: Clock,
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        let opts = Self::build_clean_options(clock, &batch);
        let resolved_log_dir = Self::resolved_log_dir(&opts.journal_base_dir);
        let existing_logs = list_log_ids(&resolved_log_dir);
        let report = Self::run_clean(
            batch.preview_token.clone(),
            batch.validated_items,
            opts,
            &mut on_progress,
        )?;
        let log_id = detect_new_log_id(&resolved_log_dir, &existing_logs);
        Ok(Self::summarize_clean(report, log_id))
    }

    fn build_clean_options<C: Clock>(clock: &C, batch: &ValidatedCleanBatch) -> CleanOptions {
        let now = UNIX_EPOCH + Duration::from_secs(clock.now_unix_secs());
        CleanOptions {
            preview_estimated_bytes: batch.preview_estimated_bytes,
            min_age: Duration::from_secs(u64::from(batch.min_age_hours) * 3600),
            now,
            journal_base_dir: batch.journal_base_dir.clone(),
        }
    }

    fn resolved_log_dir(journal_base_dir: &Option<PathBuf>) -> PathBuf {
        let log_dir = journal_base_dir
            .clone()
            .unwrap_or_else(sweeper_core::journal::default_logs_base_dir);
        sweeper_core::journal::resolve_log_dir(&log_dir)
    }

    fn run_clean<F>(
        preview_token: String,
        validated_items: Vec<ValidatedItem>,
        opts: CleanOptions,
        on_progress: &mut F,
    ) -> Result<sweeper_core::remover::CleanReport, ContractError>
    where
        F: FnMut(sweeper_contract::CleanProgressEvent),
    {
        remover::execute_clean_with_progress(validated_items, opts, |p: CleanProgress| {
            on_progress(sweeper_contract::CleanProgressEvent {
                preview_token: preview_token.clone(),
                target_id: p.target_id,
                completed_items: U64String(p.completed_items),
                total_items: U64String(p.total_items),
                bytes_freed_so_far: U64String(p.bytes_freed_so_far),
                status: p.status,
            });
        })
        .map_err(|e| ContractError::Internal {
            reason: sanitize_reason(&e.to_string()),
        })
    }

    fn summarize_clean(
        report: sweeper_core::remover::CleanReport,
        log_id: Option<String>,
    ) -> ExecuteCleanResponse {
        let deleted_count = report
            .items
            .iter()
            .filter(|i| i.status == "deleted")
            .count() as u64;
        let skipped_reasons = Self::collect_skipped_reasons(&report.items);
        ExecuteCleanResponse {
            bytes_freed: U64String(report.bytes_freed),
            deleted_count: U64String(deleted_count),
            skipped_count: U64String(report.skipped_count),
            skipped_reasons,
            aborted: report.aborted,
            abort_reason: report.abort_reason,
            log_id,
            journal_errors: report.journal_errors,
        }
    }

    fn collect_skipped_reasons(
        items: &[sweeper_core::remover::ItemCleanResult],
    ) -> Vec<SkippedReasonDto> {
        let mut reasons: BTreeMap<String, u64> = BTreeMap::new();
        for item in items {
            if item.status == "Skipped" || item.status == "aborted" {
                let key = if item.reason.trim().is_empty() {
                    "Skipped".to_string()
                } else {
                    item.reason.clone()
                };
                *reasons.entry(key).or_insert(0) += 1;
            }
        }
        reasons
            .into_iter()
            .map(|(reason, count)| SkippedReasonDto {
                reason,
                count: U64String(count),
            })
            .collect()
    }
}
