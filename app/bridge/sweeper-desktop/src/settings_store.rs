//! Settings store with safe defaults fallback and untrusted-input sanitization.

use std::path::PathBuf;

use sweeper_contract::SettingsDto;
use sweeper_core::settings::{self, SweeperSettings};

/// Settings store (loads/sanitizes JSON; invalid/missing/tampered => safe defaults).
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    settings: SweeperSettings,
    /// Monotonic generation incremented on scan/analyzer/clean-affecting changes to invalidate sessions.
    generation: u64,
    /// Last non-fatal sanitization warnings from load/parse (best-effort diagnostics).
    warnings: Vec<String>,
}

impl SettingsStore {
    /// Create with safe defaults and no backing file.
    pub fn new_default() -> Self {
        Self {
            path: None,
            settings: SweeperSettings::default(),
            generation: 0,
            warnings: Vec::new(),
        }
    }

    /// Create with a backing file path; loads and sanitizes or falls back to defaults.
    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            path: Some(path),
            settings: SweeperSettings::default(),
            generation: 0,
            warnings: Vec::new(),
        };
        s.reload();
        s
    }

    /// Reload from disk, falling back to safe defaults on any error/invalid data.
    /// Uses `sweeper_core` untrusted settings re-validation/sanitization.
    pub fn reload(&mut self) {
        let sanitized = self
            .path
            .as_ref()
            .map(|p| settings::load_untrusted_settings(p))
            .unwrap_or_else(|| settings::parse_untrusted_settings_json("{}"));
        self.settings = sanitized.settings;
        self.warnings = sanitized.warnings;
    }

    /// Get current validated settings as a contract DTO.
    pub fn get(&self) -> SettingsDto {
        to_dto(&self.settings)
    }

    /// Get current validated core settings.
    pub fn core(&self) -> SweeperSettings {
        self.settings.clone()
    }

    /// Last non-fatal sanitization warnings from load/parse.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Update settings from an untrusted DTO-like payload after sanitization and persist if configured.
    /// Returns sanitized settings DTO. Increments generation if scan/analyzer/clean-affecting fields changed.
    pub fn update(&mut self, requested: SettingsDto) -> SettingsDto {
        let sanitized = settings::parse_untrusted_settings_json(
            &serde_json::to_string(&requested).unwrap_or_else(|_| "{}".to_string()),
        );
        if scan_affecting_changed(&self.settings, &sanitized.settings) {
            self.generation = self.generation.saturating_add(1);
        }
        self.settings = sanitized.settings;
        self.warnings = sanitized.warnings;
        if let Some(path) = &self.path {
            let _ = settings::persist_settings_json(path, &self.settings);
        }
        to_dto(&self.settings)
    }

    /// Current settings generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

fn to_dto(settings: &SweeperSettings) -> SettingsDto {
    SettingsDto {
        enable_tier1: settings.enable_tier1,
        enable_tier2: settings.enable_tier2,
        enable_tier3: settings.enable_tier3,
        min_age_hours: settings.min_age_hours,
        allow_delete_unknown: settings.allow_delete_unknown,
        custom_paths: settings.custom_paths.clone(),
        stale_project_days: settings.stale_project_days,
    }
}

fn scan_affecting_changed(old: &SweeperSettings, new: &SweeperSettings) -> bool {
    old.enable_tier1 != new.enable_tier1
        || old.enable_tier2 != new.enable_tier2
        || old.enable_tier3 != new.enable_tier3
        || old.min_age_hours != new.min_age_hours
        || old.allow_delete_unknown != new.allow_delete_unknown
        || old.custom_paths != new.custom_paths
        || old.stale_project_days != new.stale_project_days
}
