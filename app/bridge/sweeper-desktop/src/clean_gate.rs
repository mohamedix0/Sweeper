//! Coordination of in-flight `Clean` operations for graceful shutdown.
//!
//! `execute_clean` holds a guard for the whole removal. The Elevate restart
//! path waits for the guard count to reach zero before it hands over the
//! single-instance guard, launches the elevated process, or exits, so an
//! in-flight `Clean` is never terminated mid-run.

use std::sync::{Arc, Condvar, Mutex};

/// Tracks how many `Clean` operations are currently in flight.
#[derive(Debug, Clone, Default)]
pub struct CleanGate {
    inner: Arc<(Mutex<usize>, Condvar)>,
}

/// RAII token held by one in-flight `Clean`. Dropping it marks the Clean settled.
#[derive(Debug)]
pub struct CleanGuard {
    inner: Arc<(Mutex<usize>, Condvar)>,
}

impl CleanGate {
    /// Register the start of a Clean. The returned guard must be held until the
    /// Clean settles; dropping it wakes any graceful-exit waiter.
    pub fn enter(&self) -> CleanGuard {
        let mut active = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
        *active += 1;
        CleanGuard {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Block until no Clean is in flight. A poisoned lock is recovered rather
    /// than propagated so shutdown can still complete.
    pub fn wait_for_idle(&self) {
        let (lock, cv) = self.inner.as_ref();
        let mut active = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *active != 0 {
            active = cv.wait(active).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Number of Cleans currently in flight (diagnostics/tests).
    pub fn active_count(&self) -> usize {
        *self.inner.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for CleanGuard {
    fn drop(&mut self) {
        let (lock, cv) = self.inner.as_ref();
        let mut active = lock.lock().unwrap_or_else(|e| e.into_inner());
        if *active > 0 {
            *active -= 1;
        }
        if *active == 0 {
            cv.notify_all();
        }
    }
}
