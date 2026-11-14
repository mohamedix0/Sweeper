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

#[test]
fn elevated_scan_discovers_and_cleans_sandboxed_elevation_target_contents_only() {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use sweeper_core::paths::CanonicalPath;
    use sweeper_core::remover::{self, CleanOptions, ValidatedItem};
    use sweeper_knowledge::{Risk, Tier};
    use sweeper_testkit::Sandbox;

    fn sandbox_base_dir() -> PathBuf {
        let base = if let Some(v) = std::env::var_os("CARGO_TARGET_DIR") {
            PathBuf::from(v).join("sweeper-test-sandboxes")
        } else {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            manifest
                .join("../../target/sweeper-test-sandboxes")
                .components()
                .collect::<PathBuf>()
        };
        let _ = fs::create_dir_all(&base);
        std::fs::canonicalize(&base).unwrap_or(base)
    }

    fn sandbox_in_target() -> Sandbox {
        Sandbox::new_in(sandbox_base_dir()).unwrap()
    }

    fn old_mtime_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(48 * 3600)
    }

    fn recent_mtime_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(3600)
    }

    fn set_mtime(path: &Path, unix_secs: u64) {
        let ft = sweeper_testkit::filetime_from_unix_secs(unix_secs);
        sweeper_testkit::set_last_write_time(path, ft).unwrap();
    }

    let sb = sandbox_in_target();
    let elevated_root = sb.create_dir(r"elev\windows-temp").unwrap();
    let outside_canary_parent = sb.create_dir(r"outside\canary").unwrap();
    // Sentinel sandbox only; never real system paths.
    assert!(!elevated_root
        .to_string_lossy()
        .to_ascii_uppercase()
        .starts_with(r"C:\WINDOWS"));
    assert!(!elevated_root
        .to_string_lossy()
        .to_ascii_uppercase()
        .starts_with(r"C:\PROGRAMDATA"));

    let old_file = elevated_root.join("old-eligible.tmp");
    fs::write(&old_file, b"old-eligible-bytes").unwrap();
    set_mtime(&old_file, old_mtime_secs());

    let recent_file = elevated_root.join("recent-skip.tmp");
    fs::write(&recent_file, b"recent").unwrap();
    set_mtime(&recent_file, recent_mtime_secs());

    let canary_file = outside_canary_parent.join("canary.txt");
    fs::write(&canary_file, b"canary-outside").unwrap();
    let link_path = elevated_root.join("escape-link");
    sweeper_testkit::create_junction(&link_path, &outside_canary_parent).unwrap();

    fn strip_extended_prefix(p: PathBuf) -> PathBuf {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest.to_string());
        }
        p
    }

    fn canon_str(p: &Path) -> String {
        let stripped = strip_extended_prefix(p.to_path_buf());
        CanonicalPath::from_existing_path(&stripped.to_string_lossy())
            .unwrap()
            .as_str()
            .to_string()
    }

    let mut overrides = BTreeMap::new();
    overrides.insert("windows-temp".to_string(), canon_str(&elevated_root));

    let settings = SweeperSettings {
        enable_tier1: true,
        enable_tier2: false,
        enable_tier3: true,
        enable_elevation_targets: true,
        min_age_hours: 24,
        ..SweeperSettings::default()
    };
    let report = scanner::scan_with_settings_options_and_elevation(
        &settings,
        ScanSettingsOptions {
            elevation_target_root_overrides: overrides,
            ..ScanSettingsOptions::default()
        },
        ScanElevationContext {
            process_elevated: true,
        },
    )
    .unwrap();

    // Even with process_elevated: true, a Tier1/Safe non-elevation target behaves unchanged.
    assert!(
        report
            .targets
            .iter()
            .any(|t| t.target_id == "user-temp" && t.tier == Tier::Tier1 && t.risk == Risk::Safe),
        "expected Tier1/Safe non-elevation target user-temp to remain present/unchanged: {:?}",
        report
            .targets
            .iter()
            .map(|t| (&t.target_id, t.tier, t.risk))
            .collect::<Vec<_>>()
    );

    let elev = report
        .targets
        .iter()
        .find(|t| t.target_id == "windows-temp")
        .expect("elevation target discovered via override");
    assert!(elev.scanned, "status: {:?}", elev.status_reason);
    assert_eq!(elev.tier, Tier::Tier3);
    assert_eq!(elev.risk, Risk::Caution);

    let old_name = old_file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_ascii_lowercase();
    let recent_name = recent_file
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_ascii_lowercase();
    let old_item = elev
        .items
        .iter()
        .find(|i| i.path.to_ascii_lowercase().contains(&old_name))
        .expect("eligible old file discovered");
    assert!(old_item.age_eligible, "old file should be eligible");
    assert!(
        !elev.items.iter().any(|i| {
            i.path.to_ascii_lowercase().contains(&recent_name) && i.age_eligible
        }),
        "recent file (< min_age_hours) inside elevated target must be ineligible/skipped for cleaning: {:?}",
        elev.items
    );
    assert!(
        elev.skipped_reasons.contains_key("LinkPoint") || elev.skipped_count >= 1,
        "reparse link should be skipped: skipped_reasons={:?} skipped_count={} diagnostics={:?}",
        elev.skipped_reasons,
        elev.skipped_count,
        elev.skipped_diagnostics
    );

    let root_canon = canon_str(&elevated_root);
    let validated = elev
        .items
        .iter()
        .filter(|i| i.age_eligible)
        .map(|i| {
            ValidatedItem::from_allowlisted_scan(
                i.id.clone(),
                "windows-temp",
                root_canon.clone(),
                i.path.clone(),
                i.size,
                Some(i.mtime_unix_secs),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(
        !validated.is_empty(),
        "expected eligible items to clean contents-only"
    );

    let journal_dir = sb.create_dir("journal-elev").unwrap();
    let clean = remover::execute_clean(
        validated,
        CleanOptions {
            preview_estimated_bytes: elev.eligible_bytes.max(1),
            min_age: Duration::from_secs(24 * 3600),
            now: SystemTime::now(),
            journal_base_dir: Some(journal_dir),
        },
    )
    .unwrap();
    assert!(!clean.aborted, "clean aborted: {:?}", clean.abort_reason);
    assert!(
        elevated_root.exists(),
        "target root directory must remain intact"
    );
    assert!(!old_file.exists(), "eligible old file should be cleaned");
    assert!(
        recent_file.exists(),
        "recent file inside elevated target should remain (age-skipped/ineligible)"
    );
    assert!(
        canary_file.exists(),
        "outside canary file must remain untouched"
    );
    assert!(
        link_path.exists(),
        "link entry itself may remain as skipped reparse point"
    );
}
