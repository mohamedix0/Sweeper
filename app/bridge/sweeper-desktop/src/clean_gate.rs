//! Coordination of in-flight `Clean` operations for graceful shutdown.
//!
//! `execute_clean` holds a guard for the whole removal. Before the Elevate
//! restart path hands over the single-instance guard, launches the elevated
//! process, or exits, it atomically **seals** this gate and waits for any
//! in-flight Clean to settle. Sealing prevents a new Clean from entering in the
//! window between observing an idle gate and the exit, so no Clean can be
//! terminated mid-run (#92). If the relaunch is aborted or fails, the gate is
//! unsealed and the app stays usable.

use std::sync::{Arc, Condvar, Mutex};

/// Shared gate state: in-flight Clean count plus the shutdown seal.
#[derive(Debug, Default)]
struct CleanGateState {
    active: usize,
    sealed: bool,
}

/// Tracks in-flight Cleans and the shutdown seal for graceful exit.
#[derive(Debug, Clone, Default)]
pub struct CleanGate {
    inner: Arc<(Mutex<CleanGateState>, Condvar)>,
}

/// RAII token held by one in-flight `Clean`. Dropping it marks the Clean settled.
#[derive(Debug)]
pub struct CleanGuard {
    inner: Arc<(Mutex<CleanGateState>, Condvar)>,
}

impl CleanGate {
    /// Register the start of a Clean. Returns `None` when the gate is sealed for
    /// shutdown, in which case the caller must abort without deleting anything.
    /// The returned guard must be held until the Clean settles; dropping it wakes
    /// any graceful-exit waiter.
    pub fn enter(&self) -> Option<CleanGuard> {
        let (lock, _cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if state.sealed {
            return None;
        }
        state.active += 1;
        Some(CleanGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Atomically seal the gate against new Cleans, then block until every Clean
    /// already in flight has settled. Sealing and waiting share one lock
    /// acquisition so no Clean can slip in after the idle check (#92). A poisoned
    /// lock is recovered rather than propagated so shutdown can still complete.
    pub fn seal_and_wait_for_idle(&self) {
        let (lock, cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.sealed = true;
        while state.active != 0 {
            state = cv.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Lift the shutdown seal so the app can accept Cleans again after an
    /// aborted or failed relaunch.
    pub fn unseal(&self) {
        let (lock, _cv) = self.inner.as_ref();
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.sealed = false;
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
