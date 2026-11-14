//! Elevation status, elevated relaunch, and single-instance handoff orchestration.
//!
//! Default implementations call `sweeper_core::elevation` (safe wrappers over `sweeper-winfs`).
//! Tests inject fakes to avoid real UAC prompts and OS mutex side effects.

use std::fmt;
use std::sync::Mutex;

use sweeper_contract::{ContractError, ElevationStatusDto, RelaunchResultDto};
use sweeper_core::elevation::{self, RelaunchOutcome, SingleInstanceGuard};

/// Abstraction over elevation checks and relaunch launching (DI seam for deterministic tests).
pub trait ElevationProvider: Send + Sync + fmt::Debug {
    /// Returns whether the current process token is elevated.
    fn is_elevated(&self) -> Result<bool, String>;
    /// Attempt to relaunch the current executable elevated.
    fn relaunch(&self) -> Result<RelaunchOutcome, String>;
}

/// Abstraction over acquiring the single-instance mutex (DI seam).
pub trait SingleInstanceProvider: Send + Sync + fmt::Debug {
    /// Acquire the per-user single-instance guard, waiting up to `timeout_ms`.
    fn acquire(&self, timeout_ms: u32) -> Result<SingleInstanceGuard, String>;
}

/// Real provider using Win32-backed `sweeper_core::elevation` primitives.
#[derive(Debug, Clone, Default)]
pub struct RealElevationProvider;

impl ElevationProvider for RealElevationProvider {
    fn is_elevated(&self) -> Result<bool, String> {
        elevation::is_process_elevated().map_err(|e| e.to_string())
    }

    fn relaunch(&self) -> Result<RelaunchOutcome, String> {
        elevation::relaunch_current_exe_elevated().map_err(|e| e.to_string())
    }
}

/// Real single-instance provider.
#[derive(Debug, Clone, Default)]
pub struct RealSingleInstanceProvider;

impl SingleInstanceProvider for RealSingleInstanceProvider {
    fn acquire(&self, timeout_ms: u32) -> Result<SingleInstanceGuard, String> {
        elevation::acquire_single_instance(timeout_ms).map_err(|e| e.to_string())
    }
}

/// Holder for the app's single-instance guard (may be absent in tests or if acquisition fails in dev).
#[derive(Debug, Default)]
pub struct SingleInstanceState {
    inner: Mutex<Option<SingleInstanceGuard>>,
}

impl SingleInstanceState {
    /// Create empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create state holding a guard.
    pub fn with_guard(guard: SingleInstanceGuard) -> Self {
        Self {
            inner: Mutex::new(Some(guard)),
        }
    }

    fn take(&self) -> Option<SingleInstanceGuard> {
        self.inner.lock().ok().and_then(|mut g| g.take())
    }

    fn put(&self, guard: SingleInstanceGuard) {
        if let Ok(mut g) = self.inner.lock() {
            *g = Some(guard);
        }
    }

    /// Best-effort release for graceful shutdown (not required, Drop handles it).
    pub fn drop_guard(&self) {
        let _ = self.take();
    }

    /// Returns true if a guard is currently held by this process (best-effort inspection for tests/diagnostics).
    pub fn is_held(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|guard| guard.is_held()))
            .unwrap_or(false)
    }
}

/// Execute `get_elevation_status` against a provider.
pub fn get_elevation_status(
    provider: &dyn ElevationProvider,
) -> Result<ElevationStatusDto, ContractError> {
    let is_elevated = provider
        .is_elevated()
        .map_err(|e| ContractError::Internal {
            reason: sanitize_internal(&e),
        })?;
    Ok(ElevationStatusDto { is_elevated })
}

/// Execute `relaunch_elevated` with injected providers/hooks (deterministic seam).
///
/// Behavior:
/// - If already elevated: returns `Ok { launched: false }` without invoking launcher or exit hook.
/// - If not elevated: releases single-instance guard for handoff, invokes launcher.
///   - Cancelled: re-acquires guard and returns sanitized error, no exit.
///   - Launched: returns `Ok { launched: true }` and triggers exit hook after response preparation.
pub fn relaunch_elevated(
    provider: &dyn ElevationProvider,
    single_instance: &SingleInstanceState,
    exit_app: &dyn Fn(i32),
) -> Result<RelaunchResultDto, ContractError> {
    let elevated = provider
        .is_elevated()
        .map_err(|e| ContractError::Internal {
            reason: sanitize_internal(&e),
        })?;
    if elevated {
        return Ok(RelaunchResultDto { launched: false });
    }

    let mut guard = single_instance
        .take()
        .ok_or_else(|| ContractError::Internal {
            reason: "single-instance guard unavailable".to_string(),
        })?;

    if let Err(e) = guard.release_for_handoff() {
        // Best-effort: try to restore ownership before returning error.
        let _ = guard.reacquire(5_000);
        single_instance.put(guard);
        return Err(ContractError::Internal {
            reason: sanitize_relaunch_error(&e.to_string()),
        });
    }

    match provider.relaunch() {
        Ok(RelaunchOutcome::CancelledByUser) => {
            // UAC cancelled: re-acquire guard and keep running.
            let _ = guard.reacquire(5_000);
            single_instance.put(guard);
            Err(ContractError::InvalidInput {
                reason: "elevation request was cancelled".to_string(),
            })
        }
        Ok(RelaunchOutcome::Launched) => {
            // Handoff complete for this instance; elevated instance will acquire mutex.
            // Do not put guard back. Trigger clean exit after returning launched=true.
            drop(guard);
            exit_app(0);
            Ok(RelaunchResultDto { launched: true })
        }
        Err(e) => {
            // Relaunch failed: restore guard and report sanitized error.
            let _ = guard.reacquire(5_000);
            single_instance.put(guard);
            Err(ContractError::Internal {
                reason: sanitize_relaunch_error(&e),
            })
        }
    }
}

/// Sanitize relaunch-related errors without leaking paths/stack traces.
pub fn sanitize_relaunch_error(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains("cancel") || lower.contains("1223") {
        return "elevation request was cancelled".to_string();
    }
    if lower.contains('\\') || lower.contains('/') || lower.contains("stack") {
        return "failed to relaunch elevated".to_string();
    }
    let trimmed = msg.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        "failed to relaunch elevated".to_string()
    } else {
        trimmed.to_string()
    }
}

fn sanitize_internal(msg: &str) -> String {
    let lower = msg.to_lowercase();
    if lower.contains('\\')
        || lower.contains('/')
        || lower.contains("stack")
        || lower.contains("path")
    {
        "operation failed".to_string()
    } else {
        let t = msg.trim();
        if t.is_empty() || t.len() > 120 {
            "operation failed".to_string()
        } else {
            t.to_string()
        }
    }
}

/// Default exit hook for production (Tauri/runtime will terminate process).
pub fn default_exit(code: i32) {
    std::process::exit(code);
}

/// Convenience: acquire real single-instance guard with bounded retry suitable for startup.
pub fn acquire_startup_single_instance() -> Result<SingleInstanceGuard, String> {
    // Bounded wait to hand off from a standard instance that is relaunching elevated.
    RealSingleInstanceProvider.acquire(15_000)
}
