//! Tauri desktop shell (Phase 2 read-only bridge).
//! No deletion logic, no `execute_clean`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod clock;
pub mod commands_impl;
pub mod elevation;
pub mod session;
pub mod settings_store;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use clock::{Clock, SystemClock};

use elevation::{ElevationProvider, RealElevationProvider, SingleInstanceState};
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
    /// Elevation status/relaunch provider (DI seam; tests can inject fakes).
    pub elevation_provider: Arc<dyn ElevationProvider>,
    /// Single-instance mutex guard holder for relaunch handoff.
    pub single_instance: Arc<SingleInstanceState>,
    /// App exit hook (DI seam; default exits process after successful relaunch handoff).
    pub app_exit: Arc<dyn Fn(i32) + Send + Sync>,
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
            elevation_provider: Arc::new(RealElevationProvider),
            single_instance: Arc::new(SingleInstanceState::new()),
            app_exit: Arc::new(elevation::default_exit),
        }
    }

    /// Attach a startup-acquired single-instance guard.
    pub fn with_single_instance_guard(
        mut self,
        guard: sweeper_core::elevation::SingleInstanceGuard,
    ) -> Self {
        self.single_instance = Arc::new(SingleInstanceState::with_guard(guard));
        self
    }
}

impl<C: Clock> AppState<C> {
    /// Create state with a custom clock (for tests).
    pub fn with_clock(clock: C) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(SessionStore::new())),
            settings: Arc::new(Mutex::new(SettingsStore::new_default())),
            clock,
            elevation_provider: Arc::new(RealElevationProvider),
            single_instance: Arc::new(SingleInstanceState::new()),
            app_exit: Arc::new(|_| {}),
        }
    }

    /// Override elevation provider, single-instance holder, and exit hook (test seam).
    pub fn with_elevation_test_hooks(
        mut self,
        provider: Arc<dyn ElevationProvider>,
        single_instance: Arc<SingleInstanceState>,
        app_exit: Arc<dyn Fn(i32) + Send + Sync>,
    ) -> Self {
        self.elevation_provider = provider;
        self.single_instance = single_instance;
        self.app_exit = app_exit;
        self
    }
}
