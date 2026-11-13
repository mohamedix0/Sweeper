//! Clock abstraction for TTL enforcement.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Clock trait for injectable time.
pub trait Clock: Clone + Send + Sync + 'static {
    /// Current time as unix seconds.
    fn now_unix_secs(&self) -> u64;
}

/// System clock implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs()
    }
}

/// Fixed clock for tests.
#[derive(Debug, Clone)]
pub struct FixedClock {
    now: std::sync::Arc<std::sync::Mutex<u64>>,
}

impl FixedClock {
    /// Create a fixed clock at the given unix timestamp (seconds).
    pub fn new(now_unix_secs: u64) -> Self {
        Self {
            now: std::sync::Arc::new(std::sync::Mutex::new(now_unix_secs)),
        }
    }

    /// Advance time by seconds.
    pub fn advance_secs(&self, secs: u64) {
        let mut g = self.now.lock().unwrap();
        *g = g.saturating_add(secs);
    }

    /// Set current time.
    pub fn set(&self, now_unix_secs: u64) {
        *self.now.lock().unwrap() = now_unix_secs;
    }
}

impl Clock for FixedClock {
    fn now_unix_secs(&self) -> u64 {
        *self.now.lock().unwrap()
    }
}
