//! Settings store with safe defaults fallback and untrusted-input sanitization.

use std::path::{Path, PathBuf};

use sweeper_contract::SettingsDto;
use sweeper_core::settings::{self, SweeperSettings};

/// Settings store (loads/sanitizes JSON; invalid/missing/tampered => safe defaults).
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: Option<PathBuf>,
    settings: SweeperSettings,
    /// Monotonic generation incremented ONLY on scan-affecting settings changes.
    ///
    /// Every session (Scan and Analyzer) captures this generation for
    /// lookup-time validity, so a scan-affecting change still invalidates both
    /// kinds of session. Keeping it scoped to settings that actually change what
    /// a Scan discovers means a no-op or an analyzer-only update (for example
    /// toggling `allow_delete_unknown`) leaves an in-flight ScanSession valid.
    generation: u64,
    /// Monotonic generation incremented ONLY on analyzer-affecting settings changes.
    ///
    /// Checked at Analyzer commit time in addition to `generation`, so a run
    /// that started before an analyzer-only setting toggled is rejected instead
    /// of being stored with a stale `allow_unknown`.
    analyzer_generation: u64,
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
            analyzer_generation: 0,
            warnings: Vec::new(),
        }
    }

    /// Create with a backing file path; loads and sanitizes or falls back to defaults.
    pub fn with_path(path: PathBuf) -> Self {
        let mut s = Self {
            path: Some(path),
            settings: SweeperSettings::default(),
            generation: 0,
            analyzer_generation: 0,
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
    ///
    /// Persistence happens first and its result is propagated: in-memory state
    /// and generation change only after a successful write. Returns the
    /// sanitized settings DTO on success, or the persistence error otherwise.
    pub fn update(&mut self, requested: SettingsDto) -> Result<SettingsDto, String> {
        self.update_with(requested, |path, settings| {
            settings::persist_settings_json(path, settings)
        })
    }

    /// Testable core of [`SettingsStore::update`].
    ///
    /// `write` is the persistence seam: production supplies
    /// [`settings::persist_settings_json`]; tests inject a failing writer to
    /// prove write errors propagate and in-memory state stays unchanged.
    fn update_with<F>(&mut self, requested: SettingsDto, write: F) -> Result<SettingsDto, String>
    where
        F: FnOnce(&Path, &SweeperSettings) -> Result<(), String>,
    {
        let sanitized = settings::parse_untrusted_settings_json(
            &serde_json::to_string(&requested).unwrap_or_else(|_| "{}".to_string()),
        );

        // Persist FIRST. A failed write is surfaced to the caller and must leave
        // the generation and every in-memory field untouched.
        if let Some(path) = &self.path {
            write(path.as_path(), &sanitized.settings)?;
        }

        // Publish the new state and bump only the generations whose affected
        // surface actually changed, AFTER a successful write. The scan
        // generation gates lookup-time validity for every session; the analyzer
        // generation additionally gates Analyzer commit so a run started under
        // a stale `allow_unknown` cannot be stored.
        if scan_affecting_changed(&self.settings, &sanitized.settings) {
            self.generation = self.generation.saturating_add(1);
        }
        if analyzer_affecting_changed(&self.settings, &sanitized.settings) {
            self.analyzer_generation = self.analyzer_generation.saturating_add(1);
        }
        self.settings = sanitized.settings;
        self.warnings = sanitized.warnings;
        Ok(to_dto(&self.settings))
    }

    /// Current scan-affecting settings generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Current analyzer-affecting settings generation.
    pub fn analyzer_generation(&self) -> u64 {
        self.analyzer_generation
    }
}

fn to_dto(settings: &SweeperSettings) -> SettingsDto {
    SettingsDto {
        enable_tier1: settings.enable_tier1,
        enable_tier2: settings.enable_tier2,
        enable_tier3: settings.enable_tier3,
        enable_elevation_targets: settings.enable_elevation_targets,
        min_age_hours: settings.min_age_hours,
        allow_delete_unknown: settings.allow_delete_unknown,
        custom_paths: settings.custom_paths.clone(),
        stale_project_days: settings.stale_project_days,
    }
}

/// Whether the change alters what a `Scan` discovers, and therefore must
/// invalidate an in-flight `ScanSession` immediately.
///
/// Every field the scanner consumes is included: enabled Tiers, elevation
/// targets, minimum age, custom paths and the Tier 3 staleness window.
/// `allow_delete_unknown` is deliberately excluded: it gates Clean/Analyzer
/// selection, not Scan discovery, so changing it must not invalidate a running
/// Scan. It is still persisted and published to the UI like any other field.
fn scan_affecting_changed(old: &SweeperSettings, new: &SweeperSettings) -> bool {
    old.enable_tier1 != new.enable_tier1
        || old.enable_tier2 != new.enable_tier2
        || old.enable_tier3 != new.enable_tier3
        || old.enable_elevation_targets != new.enable_elevation_targets
        || old.min_age_hours != new.min_age_hours
        || old.custom_paths != new.custom_paths
        || old.stale_project_days != new.stale_project_days
}

/// Whether the change alters what an `Analyzer` run produces, and therefore
/// must invalidate an in-flight `AnalyzerSession` immediately.
///
/// `allow_delete_unknown` feeds `AnalyzerOptions::allow_unknown` when a run
/// starts. Without its own generation bump, toggling it mid-run would leave the
/// Analyzer run (and any session it commits) valid with a stale `allow_unknown`.
/// Scan-affecting settings need no entry here: analyzer sessions capture the
/// scan generation too and are already rejected when that generation changes.
fn analyzer_affecting_changed(old: &SweeperSettings, new: &SweeperSettings) -> bool {
    old.allow_delete_unknown != new.allow_delete_unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changed_request(from: &SettingsDto) -> SettingsDto {
        let mut requested = from.clone();
        requested.enable_tier3 = !from.enable_tier3;
        requested.min_age_hours = if from.min_age_hours == 10 { 11 } else { 10 };
        requested
    }

    /// An injected write failure is reported to the caller, and neither the
    /// generation nor any in-memory state is touched when persistence fails.
    #[test]
    fn failed_write_propagates_error_and_preserves_generation_and_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = SettingsStore::with_path(dir.path().join("settings.json"));
        let initial = store.get();
        let initial_core = store.core();
        let initial_gen = store.generation();
        let initial_analyzer_gen = store.analyzer_generation();
        let initial_warnings = store.warnings().to_vec();

        let err = store
            .update_with(changed_request(&initial), |_, _| {
                Err("injected write failure".to_string())
            })
            .expect_err("injected write failure must propagate");

        assert_eq!(err, "injected write failure");
        assert_eq!(
            store.generation(),
            initial_gen,
            "generation must not bump on a failed write"
        );
        assert_eq!(
            store.analyzer_generation(),
            initial_analyzer_gen,
            "analyzer generation must not bump on a failed write"
        );
        assert_eq!(
            store.get(),
            initial,
            "in-memory DTO state must not change on a failed write"
        );
        assert_eq!(
            store.core(),
            initial_core,
            "in-memory core state must not change on a failed write"
        );
        assert_eq!(
            store.warnings(),
            initial_warnings.as_slice(),
            "warnings must not change on a failed write"
        );
    }

    /// `allow_delete_unknown` gates Clean/Analyzer selection, not Scan
    /// discovery. Persisting a change to it must update state without bumping
    /// the generation, so an in-flight ScanSession stays valid.
    #[test]
    fn non_scan_affecting_change_updates_state_without_bumping_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = SettingsStore::with_path(dir.path().join("settings.json"));
        let initial = store.get();
        let initial_gen = store.generation();
        let initial_analyzer_gen = store.analyzer_generation();
        let mut requested = initial.clone();
        requested.allow_delete_unknown = !requested.allow_delete_unknown;

        let updated = store.update(requested.clone()).expect("write succeeds");

        assert_eq!(updated, requested);
        assert_eq!(store.get(), requested);
        assert_eq!(
            store.generation(),
            initial_gen,
            "a non-scan-affecting change must not bump the scan generation"
        );
        assert_eq!(
            store.analyzer_generation(),
            initial_analyzer_gen + 1,
            "an analyzer-affecting change must bump the analyzer generation"
        );
    }

    /// A scan-affecting change advances only the scan generation; analyzer
    /// sessions are already invalidated by that generation at lookup/commit
    /// time, so no separate analyzer bump is needed.
    #[test]
    fn scan_affecting_change_leaves_analyzer_generation_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = SettingsStore::with_path(dir.path().join("settings.json"));
        let initial = store.get();
        let initial_analyzer_gen = store.analyzer_generation();

        store
            .update(changed_request(&initial))
            .expect("write succeeds");

        assert_eq!(store.analyzer_generation(), initial_analyzer_gen);
    }

    /// A successful write persists first, bumps the generation once, and leaves
    /// in-memory state equal to what was actually written to disk.
    #[test]
    fn successful_write_bumps_generation_and_matches_persisted_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        let mut store = SettingsStore::with_path(path.clone());
        let initial = store.get();
        let initial_gen = store.generation();

        let requested = changed_request(&initial);
        let updated = store.update(requested.clone()).expect("write succeeds");

        assert_eq!(updated, requested);
        assert_eq!(store.generation(), initial_gen + 1);
        assert_eq!(store.get(), requested);

        let raw = std::fs::read_to_string(&path).expect("read persisted settings");
        let persisted: SweeperSettings =
            serde_json::from_str(&raw).expect("parse persisted settings");
        assert_eq!(
            store.core(),
            persisted,
            "in-memory settings must match what was persisted"
        );
    }
}
