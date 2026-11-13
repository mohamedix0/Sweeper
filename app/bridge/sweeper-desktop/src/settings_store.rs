//! Settings store with safe defaults fallback.

use std::path::PathBuf;

use sweeper_contract::SettingsDto;
use sweeper_core::settings;

/// Settings store (loads/saves JSON; invalid/missing => safe defaults).
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    settings: SettingsDto,
    /// Monotonic generation incremented on scan-affecting changes to invalidate sessions.
    generation: u64,
}

impl SettingsStore {
    /// Create with safe defaults and no backing file.
    pub fn new_default() -> Self {
        Self {
            path: None,
            settings: SettingsDto::default(),
            generation: 0,
        }
    }

    /// Create with a backing file path; loads and validates or falls back to defaults.
    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            path: Some(path),
            settings: SettingsDto::default(),
            generation: 0,
        };
        s.reload();
        s
    }

    /// Reload from disk, falling back to safe defaults on any error/invalid data.
    pub fn reload(&mut self) {
        self.settings = self
            .path
            .as_ref()
            .and_then(|p| settings::load_settings_json::<SettingsDto>(p))
            .map(sanitize_loaded_settings)
            .unwrap_or_default();
    }

    /// Get current validated settings.
    pub fn get(&self) -> SettingsDto {
        self.settings.clone()
    }

    /// Update settings (already validated by caller) and persist if path configured.
    /// Returns new settings. Increments generation if scan-affecting fields changed.
    pub fn update(&mut self, validated: SettingsDto) -> SettingsDto {
        if scan_affecting_changed(&self.settings, &validated) {
            self.generation = self.generation.saturating_add(1);
        }
        self.settings = validated.clone();
        if let Some(path) = &self.path {
            let _ = settings::persist_settings_json(path, &validated);
        }
        validated
    }

    /// Current settings generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

fn sanitize_loaded_settings(mut s: SettingsDto) -> SettingsDto {
    if s.min_age_hours > settings::MAX_MIN_AGE_HOURS {
        s.min_age_hours = 24;
    }
    s
}

fn scan_affecting_changed(old: &SettingsDto, new: &SettingsDto) -> bool {
    old.enable_tier1 != new.enable_tier1
        || old.enable_tier2 != new.enable_tier2
        || old.enable_tier3 != new.enable_tier3
        || old.min_age_hours != new.min_age_hours
        || old.allow_delete_unknown != new.allow_delete_unknown
}
