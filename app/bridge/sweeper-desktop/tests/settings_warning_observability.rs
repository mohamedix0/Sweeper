//! Issue #146 — settings sanitization warnings must be observable on the
//! production settings command path, not only in tests.
//!
//! The data-loss framing of the original audit finding is out of scope: direct
//! truncate-write is intentional and fail-safe defaults are spec-mandated.
//! This test only proves the observability residue is fixed.

use sweeper_desktop::clock::FixedClock;
use sweeper_desktop::settings_store::SettingsStore;
use sweeper_desktop::{commands_impl, AppState};

#[test]
fn get_settings_exposes_store_sanitization_warnings() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("settings.json");
    // Tampered but syntactically valid JSON: an invalid flag, an out-of-range
    // number and a protected-path override. The loader sanitizes all three and
    // records non-fatal warnings while preserving safe defaults.
    std::fs::write(
        &path,
        r#"{
            "enable_tier1": "nope",
            "min_age_hours": 999999,
            "allow_delete_unknown": "bad",
            "protected_paths": ["C:\\Windows"]
        }"#,
    )
    .expect("write tampered settings");

    let state = AppState::with_clock(FixedClock::new(1_800_000_000));
    *state.settings.lock().expect("lock settings") = SettingsStore::with_path(path);

    // Production command path (the same one the Tauri shell invokes).
    let dto = commands_impl::get_settings(&state).expect("get_settings succeeds");

    assert!(
        !dto.warnings.is_empty(),
        "get_settings must surface the store's sanitization warnings: {dto:?}"
    );
    assert!(
        dto.warnings.iter().any(|w| w.contains("enable_tier1")),
        "the field-specific warning must be carried through: {:?}",
        dto.warnings
    );
    // Sanitization still falls back to safe defaults (do not change this).
    assert!(
        dto.enable_tier1,
        "invalid flag must fall back to a safe default"
    );
    assert_eq!(dto.min_age_hours, 24, "out-of-range value must fall back");
}
