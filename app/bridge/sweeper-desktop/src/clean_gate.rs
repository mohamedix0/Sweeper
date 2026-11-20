//! Coordination of in-flight `Clean` operations for graceful shutdown.
//!
//! `execute_clean` holds a guard for the whole removal. Before the Elevate
//! restart path hands over the single-instance guard, launches the elevated
//! process, or exits, it atomically **seals** this gate and waits for any
//! in-flight Clean to settle. Sealing prevents a new Clean from entering in the
//! window between observing an idle gate and the exit, so no Clean can be
//! terminated mid-run (#92). If the relaunch is aborted or fails, the gate is
//! unsealed and the app stays usable.
//!
//! The seal has an **owner** (see [`SealReason`]). A relaunch owns a reversible
//! seal: cancelling it lifts only its own seal. An approved window close owns a
//! terminal seal: it can never be lifted, so a relaunch cancelled *after* a
//! close approval cannot reopen the gate before `app.exit(0)` (#93).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Who owns the shutdown seal. `Close` outranks `Relaunch`, so a close approval
/// can upgrade a pending relaunch seal but a relaunch can never downgrade a
/// close seal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SealReason {
    Relaunch,
    Close,
}

/// Shared gate state: in-flight Clean count plus the owned shutdown seal.
#[derive(Debug, Default)]
struct CleanGateState {
    active: usize,
    seal: Option<SealReason>,
}

/// Tracks in-flight Cleans and the shutdown seal for graceful exit.
#[derive(Debug, Clone, Default)]
pub struct CleanGate {
    inner: Arc<(Mutex<CleanGateState>, Condvar)>,
    /// Set while a relaunch owns the seal. A concurrent relaunch is rejected
    /// before it can seal or unseal the gate (#92).
    relaunch_active: Arc<AtomicBool>,
}

/// RAII token held by one in-flight `Clean`. Dropping it marks the Clean settled.
#[derive(Debug)]
pub struct CleanGuard {
    inner: Arc<(Mutex<CleanGateState>, Condvar)>,
}

/// RAII owner of a graceful-exit relaunch seal. Only the owning call may lift
/// the seal, so a second concurrent relaunch cannot open the gate while the
/// first call's UAC prompt is pending (#92).
#[derive(Debug)]
pub struct RelaunchSealGuard {
    gate: CleanGate,
    /// Whether dropping this guard lifts the seal. Cleared on a successful
    /// handoff, where this instance exits with the gate still sealed.
    owns_seal: bool,
    /// Whether dropping this guard permits a later relaunch. Kept on
    /// abort/failure so the app stays usable; cleared on a successful handoff.
    release_flag: bool,
}

impl RelaunchSealGuard {
    /// Keep the gate sealed for process exit. A successful handoff neither
    /// unseals nor allows another relaunch in this instance.
    pub fn persist_for_exit(&mut self) {
        self.owns_seal = false;
        self.release_flag = false;
    }
}

impl Drop for RelaunchSealGuard {
    fn drop(&mut self) {
        if self.owns_seal {
            self.gate.unseal_relaunch();
        }
        if self.release_flag {
            self.gate.relaunch_active.store(false, Ordering::SeqCst);
        }
    }
}

impl CleanGate {
    /// Register the start of a Clean. Returns `None` when the gate is sealed for
    /// shutdown, in which case the caller must abort without deleting anything.
    /// The returned guard must be held until the Clean settles; dropping it wakes
    /// any graceful-exit waiter.
    pub fn enter(&self) -> Option<CleanGuard> {
        let (lock, _cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if state.seal.is_some() {
            return None;
        }
        state.active += 1;
        Some(CleanGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Atomically apply a seal, then block until every Clean already in flight
    /// has settled. Sealing and waiting share one lock acquisition so no Clean
    /// can slip in after the idle check (#92). A `Close` seal outranks a
    /// `Relaunch` seal and is therefore never downgraded by a concurrent
    /// relaunch. A poisoned lock is recovered so shutdown can still complete.
    fn seal_and_wait_for_idle(&self, reason: SealReason) {
        let (lock, cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.seal = state.seal.max(Some(reason));
        while state.active != 0 {
            state = cv.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Seal the gate on behalf of an approved window close and wait for in-flight
    /// Cleans. The seal is terminal: there is no public path that lifts it, so
    /// the process can never accept a new Clean between here and `app.exit(0)`.
    pub fn seal_for_close_and_wait(&self) {
        self.seal_and_wait_for_idle(SealReason::Close);
    }

    /// Claim the single relaunch slot and seal the gate before waiting for
    /// in-flight Cleans. Returns `None` when another relaunch is already in
    /// progress, so that caller aborts *before* sealing or touching the guard.
    /// The returned guard owns the seal: only it can lift the seal, preventing
    /// a concurrent caller from opening the gate during a pending UAC prompt.
    pub fn try_begin_relaunch(&self) -> Option<RelaunchSealGuard> {
        if self
            .relaunch_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }
        self.seal_and_wait_for_idle(SealReason::Relaunch);
        Some(RelaunchSealGuard {
            gate: self.clone(),
            owns_seal: true,
            release_flag: true,
        })
    }

    /// Lift a relaunch-owned seal so the app can accept Cleans again after an
    /// aborted or failed relaunch. A `Close` seal is deliberately left intact:
    /// only a relaunch may be reversed; a close approval persists until exit.
    fn unseal_relaunch(&self) {
        let (lock, _cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if state.seal == Some(SealReason::Relaunch) {
            state.seal = None;
        }
    }
}

impl Drop for CleanGuard {
    fn drop(&mut self) {
        let (lock, cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if state.active > 0 {
            state.active -= 1;
        }
        if state.active == 0 {
            cv.notify_all();
        }
    }
}
