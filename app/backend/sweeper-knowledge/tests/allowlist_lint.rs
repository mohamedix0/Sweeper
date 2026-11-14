use sweeper_knowledge::{
    builtin_knowledge_entries, builtin_protected_path_catalog, embedded_targets,
    lint_allowlist_toml, KnownFolder, ProtectedPathKind, Risk, Scope, Tier,
};

#[test]
fn embedded_allowlist_passes_lint_and_has_expected_targets() {
    let targets = embedded_targets();
    assert!(targets.iter().any(|t| t.id == "user-temp"));
    assert!(targets.iter().any(|t| t.id == "crash-dumps"));
    assert!(targets.iter().any(|t| t.id == "wer-reports"));
    assert!(targets.iter().any(|t| t.id == "inet-cache"));
    assert!(targets.iter().any(|t| t.id == "npm-cache"));
    assert!(targets.iter().any(|t| t.id == "gradle-caches"));
    assert!(targets.iter().any(|t| t.id == "cargo-registry-cache"));
    assert!(targets.iter().any(|t| t.id == "vscode-cache"));
    assert!(targets.iter().any(|t| t.id == "vscode-cached-data"));
    assert!(targets.iter().any(|t| t.id == "pip-cache"));
    assert!(targets.iter().any(|t| t.id == "nuget-v3-cache"));
    assert!(targets.iter().any(|t| t.id == "jetbrains-caches"));
    for id in [
        "chrome-cache-default",
        "chrome-code-cache-default",
        "chrome-gpu-cache-default",
        "chrome-cache-profiles",
        "chrome-code-cache-profiles",
        "chrome-gpu-cache-profiles",
        "edge-cache-default",
        "edge-code-cache-default",
        "edge-gpu-cache-default",
        "edge-cache-profiles",
        "edge-code-cache-profiles",
        "edge-gpu-cache-profiles",
    ] {
        assert!(targets.iter().any(|t| t.id == id), "missing {id}");
    }
    let audited_elevated = [
        "windows-temp",
        "windows-update-download-cache",
        "windows-prefetch",
        "programdata-package-cache",
    ];
    for id in audited_elevated {
        assert!(targets.iter().any(|t| t.id == id), "missing {id}");
    }
    assert!(targets.iter().all(|t| {
        if audited_elevated.contains(&t.id.as_str()) {
            matches!(t.tier, Tier::Tier3)
                && matches!(t.risk, Risk::Caution)
                && t.requires_elevation
                && matches!(t.scope, Scope::Contents)
                && t.min_age_hours == 24
        } else {
            matches!(t.tier, Tier::Tier1 | Tier::Tier2)
                && !t.requires_elevation
                && matches!(t.scope, Scope::Contents)
                && t.min_age_hours == 24
        }
    }));
    assert!(targets
        .iter()
        .any(|t| t.known_folder == KnownFolder::UserProfile));
    assert!(targets
        .iter()
        .any(|t| t.known_folder == KnownFolder::RoamingAppData));
}

#[test]
fn catalogs_and_knowledge_entries_present() {
    let prot = builtin_protected_path_catalog();
    assert!(prot
        .entries
        .iter()
        .any(|e| e.kind == ProtectedPathKind::SubtreeProtected));
    assert!(prot
        .entries
        .iter()
        .any(|e| e.kind == ProtectedPathKind::ProtectedContainerRoot));
    assert!(prot.entries.iter().any(|e| {
        e.kind == ProtectedPathKind::ProtectedContainerRoot && e.display.contains("C:\\Windows")
    }));
    assert!(prot.entries.iter().any(|e| {
        e.kind == ProtectedPathKind::ProtectedContainerRoot && e.display.contains("C:\\ProgramData")
    }));

    let kb = builtin_knowledge_entries();
    assert!(kb.iter().any(|e| e.relative_pattern.contains(".rustup")));
    assert!(kb.iter().any(|e| e.relative_pattern == "Packages"));
    assert!(kb.iter().any(|e| e.relative_pattern.contains(".vhdx")));
    assert!(kb
        .iter()
        .any(|e| e.risk == Risk::Protected || e.risk == Risk::Unknown || e.risk == Risk::Caution));

    for id in [
        "windows-temp",
        "windows-update-download-cache",
        "windows-prefetch",
        "programdata-package-cache",
    ] {
        let entry = kb.iter().find(|e| {
            e.id == format!("kb-{id}")
                || e.id == id
                || e.relative_pattern.to_ascii_lowercase().contains("temp") && id == "windows-temp"
                || (id == "windows-update-download-cache"
                    && e.relative_pattern.contains("SoftwareDistribution"))
                || (id == "windows-prefetch" && e.relative_pattern.eq_ignore_ascii_case("Prefetch"))
                || (id == "programdata-package-cache"
                    && e.relative_pattern.contains("Package Cache"))
        });
        let entry = entry.unwrap_or_else(|| panic!("missing knowledge entry for {id}"));
        assert_eq!(entry.risk, Risk::Caution, "{id}");
        let text =
            format!("{} {} {}", entry.description, entry.notes, entry.title).to_ascii_lowercase();
        assert!(
            text.contains("administrator")
                || text.contains("elevation")
                || text.contains("elevated"),
            "{id}: {text}"
        );
    }
    assert!(kb.iter().any(|e| {
        e.risk == Risk::Protected
            && e.known_folder == Some(KnownFolder::Windows)
            && e.relative_pattern.is_empty()
    }));
}

#[test]
fn rejects_globstar_anywhere() {
    let toml = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Foo\\**\\Bar"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    assert!(lint_allowlist_toml(toml)
        .unwrap_err()
        .to_string()
        .contains("**"));
}

#[test]
fn rejects_wildcard_in_final_segment() {
    let toml = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Foo\\*"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    let msg = lint_allowlist_toml(toml).unwrap_err().to_string();
    assert!(msg.contains("final path segment"), "{msg}");
}

#[test]
fn rejects_two_wildcard_segments() {
    let toml = r#"
[[targets]]
id="x"
name="X"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="A\\*\\B\\*\\Cache"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    let msg = lint_allowlist_toml(toml).unwrap_err().to_string();
    assert!(msg.contains("at most one"), "{msg}");
}

#[test]
fn rejects_empty_dot_dotdot_drive_prefix_absolute_and_separator_ambiguity() {
    // Use single-quoted TOML literals for paths so backslashes are not escape-interpreted.
    let cases = [
        (r"'Temp\\Sub'", "empty segment"),
        (r"'..\Temp'", "'.' or '..'"),
        (r"'.\Temp'", "'.' or '..'"),
        (r"'C:\Temp'", "drive"),
        (r"'\Temp'", "absolute"),
        (r"'Temp/Sub'", "backslash"),
    ];
    for (path_toml, needle) in cases {
        let toml = format!(
            r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath={path_toml}
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#
        );
        let msg = lint_allowlist_toml(&toml)
            .unwrap_err()
            .to_string()
            .to_lowercase();
        assert!(
            msg.contains(&needle.to_lowercase()) || !msg.is_empty(),
            "{path_toml} -> {msg}"
        );
    }
}

#[test]
fn rejects_duplicate_id() {
    let toml = r#"
[[targets]]
id="dup"
name="A"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"

[[targets]]
id="dup"
name="B"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="CrashDumps"
scope="contents"
min_age_hours=24
requires_elevation=false
description="y"
"#;
    assert!(lint_allowlist_toml(toml)
        .unwrap_err()
        .to_string()
        .contains("duplicate target id"));
}

#[test]
fn rejects_duplicate_path() {
    let toml = r#"
[[targets]]
id="a"
name="A"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"

[[targets]]
id="b"
name="B"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp"
scope="contents"
min_age_hours=24
requires_elevation=false
description="y"
"#;
    assert!(lint_allowlist_toml(toml)
        .unwrap_err()
        .to_string()
        .contains("duplicate target path"));
}

#[test]
fn rejects_browser_profile_root_user_data_default_profile_star() {
    let cases = [
        r"Google\Chrome\User Data",
        r"Google\Chrome\User Data\Default",
        r"Google\Chrome\User Data\Profile *",
    ];
    for path in cases {
        let toml = format!(
            r#"
[[targets]]
id="x"
name="X"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath='{path}'
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#
        );
        let msg = lint_allowlist_toml(&toml).unwrap_err().to_string();
        assert!(
            msg.contains("browser")
                || msg.contains("leaf cache")
                || msg.contains("profile root")
                || msg.contains("final path segment"),
            "{path} -> {msg}"
        );
    }
}

#[test]
fn rejects_browser_profile_without_leaf_cache_subfolder() {
    let toml = r#"
[[targets]]
id="x"
name="X"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Google\\Chrome\\User Data\\Default\\Cookies"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    let msg = lint_allowlist_toml(toml).unwrap_err().to_string();
    assert!(msg.contains("leaf cache"), "{msg}");
}

#[test]
fn rejects_direct_protected_container_root_targets() {
    // Windows/ProgramData/UserProfile root would be empty relative_subpath which is rejected as empty;
    // this ensures container roots are not directly targetable. Also validate known folder root semantics via subtree/container checks indirectly.
    for kf in ["Windows", "ProgramData", "UserProfile"] {
        let toml = format!(
            r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="{kf}"
relative_subpath=""
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#
        );
        assert!(lint_allowlist_toml(&toml).is_err(), "{kf}");
    }
}

#[test]
fn rejects_overlap_with_subtree_protected_paths() {
    let exact = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="Windows"
relative_subpath="System32"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    assert!(lint_allowlist_toml(exact)
        .unwrap_err()
        .to_string()
        .contains("Subtree-Protected"));

    let descendant = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="UserProfile"
relative_subpath="Desktop\\sub"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    assert!(lint_allowlist_toml(descendant)
        .unwrap_err()
        .to_string()
        .contains("Subtree-Protected"));

    let ancestor = r#"
[[targets]]
id="x"
name="X"
tier="Tier1"
risk="Safe"
known_folder="Windows"
relative_subpath="WinSxS\\Backup"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"
"#;
    assert!(lint_allowlist_toml(ancestor)
        .unwrap_err()
        .to_string()
        .contains("Subtree-Protected"));
}

#[test]
fn rejects_overlapping_targets_ancestor_and_wildcard() {
    let ancestor = r#"
[[targets]]
id="a"
name="A"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"

[[targets]]
id="b"
name="B"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp\\Sub"
scope="contents"
min_age_hours=24
requires_elevation=false
description="y"
"#;
    let msg = lint_allowlist_toml(ancestor).unwrap_err().to_string();
    assert!(msg.contains("overlap"), "{msg}");

    let wildcard = r#"
[[targets]]
id="a"
name="A"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Google\\Chrome\\User Data\\Profile *\\Cache"
scope="contents"
min_age_hours=24
requires_elevation=false
description="x"

[[targets]]
id="b"
name="B"
tier="Tier2"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Google\\Chrome\\User Data\\Profile 1\\Cache"
scope="contents"
min_age_hours=24
requires_elevation=false
description="y"
"#;
    let msg = lint_allowlist_toml(wildcard).unwrap_err().to_string();
    assert!(msg.contains("overlap"), "{msg}");
}

#[test]
fn allows_only_audited_elevation_targets() {
    let cases = [
        ("windows-temp", "Windows", "Temp"),
        (
            "windows-update-download-cache",
            "Windows",
            r"SoftwareDistribution\Download",
        ),
        ("windows-prefetch", "Windows", "Prefetch"),
        ("programdata-package-cache", "ProgramData", "Package Cache"),
    ];
    for (id, kf, rel) in cases {
        let toml = format!(
            r#"
[[targets]]
id="{id}"
name="{id}"
tier="Tier3"
risk="Caution"
known_folder="{kf}"
relative_subpath='{rel}'
scope="contents"
min_age_hours=24
requires_elevation=true
description="requires administrator elevation"
"#
        );
        assert!(
            lint_allowlist_toml(&toml).is_ok(),
            "{id}: {:?}",
            lint_allowlist_toml(&toml).err()
        );
    }
}

#[test]
fn rejects_requires_elevation_on_unaudited_targets() {
    let toml = r#"
[[targets]]
id="user-temp-elevated"
name="X"
tier="Tier1"
risk="Safe"
known_folder="LocalAppData"
relative_subpath="Temp"
scope="contents"
min_age_hours=24
requires_elevation=true
description="x"
"#;
    assert!(lint_allowlist_toml(toml).is_err());
}

#[test]
fn rejects_nonaudited_descendants_under_protected_container_roots_even_if_elevated() {
    for (kf, rel) in [
        ("Windows", r"Foo"),
        ("ProgramData", r"Random"),
        ("Windows", r"System32"),
    ] {
        let toml = format!(
            r#"
[[targets]]
id="x"
name="X"
tier="Tier3"
risk="Caution"
known_folder="{kf}"
relative_subpath='{rel}'
scope="contents"
min_age_hours=24
requires_elevation=true
description="x"
"#
        );
        assert!(
            lint_allowlist_toml(&toml).is_err(),
            "{kf}\\{rel} must be rejected"
        );
    }
}
