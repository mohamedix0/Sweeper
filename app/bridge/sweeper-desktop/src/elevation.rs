//! Elevation status, elevated relaunch, and single-instance handoff orchestration.
//!
//! Default implementations call `sweeper_core::elevation` (safe wrappers over `sweeper-winfs`).
//! Tests inject fakes for elevation checks/relaunch to avoid real UAC prompts;
//! single-instance acquisition is called directly (an OS mutex is unavoidable
//! for the startup handoff and has no fake consumer).

use std::fmt;
use std::sync::Mutex;

use sweeper_contract::{ContractError, ElevationStatusDto, RelaunchResultDto};
use sweeper_core::elevation::{self, RelaunchOutcome, SingleInstanceGuard};

use crate::clean_gate::CleanGate;

/// Abstraction over elevation checks and relaunch launching (DI seam for deterministic tests).
pub trait ElevationProvider: Send + Sync + fmt::Debug {
    /// Returns whether the current process token is elevated.
    fn is_elevated(&self) -> Result<bool, String>;
    /// Attempt to relaunch the current executable elevated.
    fn relaunch(&self) -> Result<RelaunchOutcome, String>;
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

    /// Remove and return the held guard, if any.
    pub fn take(&self) -> Option<SingleInstanceGuard> {
        self.inner.lock().ok().and_then(|mut g| g.take())
    }

    /// Store a guard.
    pub fn put(&self, guard: SingleInstanceGuard) {
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

/// Restore the single-instance guard after an aborted or failed relaunch,
/// leaving the still-running app fully usable. The shutdown seal is lifted by
/// dropping the `RelaunchSealGuard` owned by the calling relaunch.
fn restore_guard(guard: SingleInstanceGuard, single_instance: &SingleInstanceState) {
    let mut guard = guard;
    let _ = guard.reacquire(5_000);
    single_instance.put(guard);
}

/// Execute `relaunch_elevated` with injected providers/hooks (deterministic seam).
///
/// Behavior:
/// - If already elevated: returns `Ok { launched: false }` without invoking launcher or exit hook.
/// - Otherwise claims the single relaunch slot and atomically seals the gate
///   (blocking new Cleans) before waiting for any in-flight Clean to settle, so
///   no Clean is terminated by the restart (#92). A concurrent relaunch is
///   rejected before it can seal or unseal the gate.
/// - Then releases the single-instance guard for handoff and invokes the launcher.
///   - Cancelled: unseals the gate, re-acquires guard, returns sanitized error, no exit.
///   - Launched: returns `Ok { launched: true }` and triggers the exit hook while
///     the gate stays sealed (the process is going away).
///   - Failed: unseals the gate and restores the guard so the app stays usable.
pub fn relaunch_elevated(
    provider: &dyn ElevationProvider,
    single_instance: &SingleInstanceState,
    clean_gate: &CleanGate,
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

    // Graceful exit: claim the single relaunch slot and seal first so no new
    // Clean can enter, then let any Clean already in flight finish before
    // handing over the guard, launching, or exiting. The seal is owned by this
    // call only: a second concurrent relaunch is rejected here, before it can
    // seal or unseal the gate while this call's UAC prompt is pending (#92).
    // Sealing and waiting stay one atomic operation under the gate lock, and a
    // Clean is never cancelled or terminated.
    let seal = clean_gate
        .try_begin_relaunch()
        .ok_or_else(|| ContractError::Internal {
            reason: "relaunch already in progress".to_string(),
        })?;

    let mut guard = match single_instance.take() {
        Some(guard) => guard,
        None => {
            drop(seal);
            return Err(ContractError::Internal {
                reason: "single-instance guard unavailable".to_string(),
            });
        }
    };

    if let Err(e) = guard.release_for_handoff() {
        // Best-effort: restoring ownership and dropping the seal unseals the
        // gate and releases the relaunch slot before returning the error.
        drop(seal);
        restore_guard(guard, single_instance);
        return Err(ContractError::Internal {
            reason: sanitize_relaunch_error(&e.to_string()),
        });
    }

    match provider.relaunch() {
        Ok(RelaunchOutcome::CancelledByUser) => {
            // UAC cancelled: dropping the seal unseals the gate and releases
            // the slot; restore the guard and keep running.
            drop(seal);
            restore_guard(guard, single_instance);
            Err(ContractError::InvalidInput {
                reason: "elevation request was cancelled".to_string(),
            })
        }
        Ok(RelaunchOutcome::Launched) => {
            // Handoff complete for this instance; elevated instance will
            // acquire the mutex. Do not put the guard back. Persist the seal so
            // the process exits with the gate sealed and no further Clean may
            // start in this instance, and so no later relaunch can unseal it.
            drop(guard);
            let mut seal = seal;
            seal.persist_for_exit();
            exit_app(0);
            Ok(RelaunchResultDto { launched: true })
        }
        Err(e) => {
            // Relaunch failed: dropping the seal unseals the gate and releases
            // the slot; restore the guard so the app stays usable.
            drop(seal);
            restore_guard(guard, single_instance);
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
    elevation::acquire_single_instance(15_000).map_err(|e| e.to_string())
}
