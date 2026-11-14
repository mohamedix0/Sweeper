//! Tauri desktop shell (Phase 2 read-only bridge).
//! No deletion logic, no `execute_clean`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clock;
pub mod commands_impl;
pub mod session;
pub mod settings_store;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use clock::{Clock, SystemClock};

use session::SessionStore;
use settings_store::SettingsStore;

/// Shared application state for Tauri commands.
#[derive(Clone)]
pub struct AppState<C: Clock = SystemClock> {
    /// Session and preview token store.
    pub sessions: Arc<Mutex<SessionStore>>,
    /// Settings store.
    pub settings: Arc<Mutex<SettingsStore>>,
    /// Clock for TTL enforcement.
    pub clock: C,
}

fn default_settings_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Sweeper").join("settings.json"))
}

impl AppState<SystemClock> {
    /// Create state with system clock and persisted settings (if available).
    pub fn new_default() -> Self {
        let settings = match default_settings_path() {
            Some(path) => SettingsStore::with_path(path),
            None => SettingsStore::new_default(),
        };
        Self {
            sessions: Arc::new(Mutex::new(SessionStore::new())),
            settings: Arc::new(Mutex::new(settings)),
            clock: SystemClock,
        }
    }
}

impl<C: Clock> AppState<C> {
    /// Create state with a custom clock (for tests).
    pub fn with_clock(clock: C) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(SessionStore::new())),
            settings: Arc::new(Mutex::new(SettingsStore::new_default())),
            clock,
        }
    }
}
