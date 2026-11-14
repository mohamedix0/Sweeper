#![cfg(windows)]

use sweeper_core::scanner::{self, ScanSettingsOptions};
use sweeper_core::settings::SweeperSettings;
use sweeper_core::ScanElevationContext;

#[test]
fn elevation_settings_default_and_sanitize() {
    let s = SweeperSettings::default();
    assert!(!s.enable_elevation_targets);

    let parsed2 = SweeperSettings::parse_untrusted_json(r#"{"enable_elevation_targets": true}"#);
    assert!(parsed2.settings.enable_elevation_targets);
    assert!(parsed2.warnings.is_empty());

    let parsed3 = SweeperSettings::parse_untrusted_json(r#"{"enableElevationTargets": true}"#);
    assert!(parsed3.settings.enable_elevation_targets);

    let parsed4 = SweeperSettings::parse_untrusted_json(r#"{"enable_elevation_targets": "yes"}"#);
    assert!(!parsed4.settings.enable_elevation_targets);
    assert!(parsed4
        .warnings
        .iter()
        .any(|w| w.contains("enable_elevation_targets invalid")));
}

#[test]
fn non_elevated_excludes_requires_elevation_targets_even_if_enabled() {
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        enable_elevation_targets: true,
        ..SweeperSettings::default()
    };
    let report = scanner::scan_with_settings_options_and_elevation(
        &settings,
        ScanSettingsOptions::default(),
        ScanElevationContext {
            process_elevated: false,
        },
    )
    .unwrap();
    for t in &report.targets {
        assert_ne!(t.target_id, "windows-temp");
        assert_ne!(t.target_id, "windows-update-download-cache");
        assert_ne!(t.target_id, "windows-prefetch");
        assert_ne!(t.target_id, "programdata-package-cache");
    }
}

#[test]
fn elevated_without_elevation_targets_flag_excludes_requires_elevation_targets() {
    let settings = SweeperSettings {
        enable_tier1: false,
        enable_tier2: false,
        enable_tier3: true,
        enable_elevation_targets: false,
        ..SweeperSettings::default()
    };
    let report = scanner::scan_with_settings_options_and_elevation(
        &settings,
        ScanSettingsOptions::default(),
        ScanElevationContext {
            process_elevated: true,
        },
    )
    .unwrap();
    for t in &report.targets {
        assert_ne!(t.target_id, "windows-temp");
        assert_ne!(t.target_id, "windows-update-download-cache");
        assert_ne!(t.target_id, "windows-prefetch");
        assert_ne!(t.target_id, "programdata-package-cache");
    }
}
