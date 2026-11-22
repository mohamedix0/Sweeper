//! Issue #41 — reference parity for uv/Maven caches via safe whole-cache cleaning.
//!
//! These targets must only ever be cleanable as an entire, explicitly scoped cache
//! directory. Per-file age-based deletion of package contents is prohibited: it can
//! leave a partially cleared cache and corrupt a package manager's internal index.

use sweeper_knowledge::{
    builtin_knowledge_entries, embedded_targets, lint_allowlist_toml, Risk, Scope, Tier,
};

/// (target id, known folder, relative subpath) for the reinstated cleanable caches.
const WHOLE_CACHE_TARGETS: &[(&str, &str, &str)] = &[
    ("uv-cache", "LocalAppData", r"uv\cache"),
    ("maven-wrapper-dists", "UserProfile", r".m2\wrapper\dists"),
];

#[test]
fn whole_cache_targets_are_present_with_safe_semantics() {
    let targets = embedded_targets();
    for &(id, known_folder, subpath) in WHOLE_CACHE_TARGETS {
        let t = targets
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("missing whole-cache target `{id}`"));
        let known_folder_name = match t.known_folder {
            sweeper_knowledge::KnownFolder::LocalAppData => "LocalAppData",
            sweeper_knowledge::KnownFolder::UserProfile => "UserProfile",
            _ => "other",
        };
        assert_eq!(
            (
                known_folder_name,
                t.relative_subpath.as_str(),
                t.tier,
                t.risk,
                t.scope,
                t.min_age_hours,
                t.requires_elevation,
            ),
            (
                known_folder,
                subpath,
                Tier::Tier2,
                Risk::Caution,
                Scope::WholeCache,
                0,
                false
            ),
            "{id}"
        );
    }
}

#[test]
fn no_target_uses_whole_cache_scope_except_the_two_audited_caches() {
    let targets = embedded_targets();
    let whole_cache_ids: Vec<&str> = targets
        .iter()
        .filter(|t| t.scope == Scope::WholeCache)
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(whole_cache_ids, vec!["uv-cache", "maven-wrapper-dists"]);
}

#[test]
fn kb_annotations_mark_both_caches_cleanable_with_rationale() {
    let kb = builtin_knowledge_entries();
    let uv = kb
        .iter()
        .find(|e| e.id == "kb-uv-cache")
        .expect("missing uv Knowledge Base entry");
    let maven = kb
        .iter()
        .find(|e| e.id == "kb-maven-repository-cache")
        .expect("missing Maven Knowledge Base entry");
    let uv_text = format!("{} {}", uv.description, uv.notes).to_ascii_lowercase();
    let maven_text = format!("{} {}", maven.description, maven.notes).to_ascii_lowercase();
    let whole_cache_words = |s: &str| s.replace('-', " ").contains("whole cache");
    let checks = [
        whole_cache_words(&uv_text),
        uv_text.contains("per-file"),
        !uv_text.contains("analyzer only") && !uv_text.contains("not cleanable"),
        whole_cache_words(&maven_text),
        maven_text.contains("locally installed"),
        !maven_text.contains("analyzer only") && !maven_text.contains("not cleanable"),
    ];
    assert_eq!(checks, [true; 6], "uv={uv_text} maven={maven_text}");
}

#[test]
fn lint_rejects_whole_cache_with_age_based_deletion() {
    let toml = r#"
[[targets]]
id = "uv-cache"
name = "uv Cache"
tier = "Tier2"
risk = "Caution"
known_folder = "LocalAppData"
relative_subpath = "uv\\cache"
scope = "whole_cache"
min_age_hours = 24
requires_elevation = false
description = "bad"
"#;
    let err = lint_allowlist_toml(toml).expect_err("whole-cache + age filter must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("min_age_hours") || msg.contains("age"),
        "expected an age-related lint error, got: {msg}"
    );
}

#[test]
fn whole_cache_targets_have_distinct_leaf_scoped_roots() {
    let targets = embedded_targets();
    let a = targets.iter().find(|t| t.id == "uv-cache").unwrap();
    let b = targets
        .iter()
        .find(|t| t.id == "maven-wrapper-dists")
        .unwrap();
    let distinct_roots = (a.known_folder, a.relative_subpath.as_str())
        != (b.known_folder, b.relative_subpath.as_str());
    // The linter already rejects any overlap with Subtree-Protected Paths; the
    // embedded allowlist is linted at build time. Here we assert the two new
    // cache roots are distinct, wildcard-free, and end in an explicit cache leaf.
    let leaf_scoped = [a, b].iter().all(|t| {
        let segs: Vec<&str> = t.relative_subpath.split('\\').collect();
        let leaf = segs.last().copied().unwrap_or("").to_ascii_lowercase();
        !t.relative_subpath.contains('*') && (leaf == "cache" || leaf == "dists")
    });
    assert_eq!((distinct_roots, leaf_scoped), (true, true));
}
