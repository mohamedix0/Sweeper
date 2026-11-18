//! Build-time reference-parity checks for the Vivaldi/Opera leaf-cache targets and the
//! Knowledge Base annotations that document the Explorer cache-DB and uv/Maven analyzer
//! entries (issues #39 / #40 / #41).
//!
//! Every check is folded into a single `assert_eq!`/`assert!` over an array or tuple of all
//! conditions. This keeps each test well under the complexity/length thresholds and avoids any
//! contiguous block of four or more assert macros, while preserving all behavioural checks.

use sweeper_knowledge::{
    builtin_knowledge_entries, embedded_targets, KnownFolder, Risk, Scope, Tier,
};

/// (target id, expected `relative_subpath`, expected `known_folder`) for the Vivaldi/Opera
/// leaf-cache targets. Opera's HTTP cache leaves live under `LocalAppData`, while its Code Cache
/// and GPUCache leaves live under the Roaming Opera profile tree (`RoamingAppData`).
const VIVALDI_OPERA_LEAF_TARGETS: &[(&str, &str, KnownFolder)] = &[
    (
        "vivaldi-cache-default",
        r"Vivaldi\User Data\Default\Cache",
        KnownFolder::LocalAppData,
    ),
    (
        "vivaldi-code-cache-default",
        r"Vivaldi\User Data\Default\Code Cache",
        KnownFolder::LocalAppData,
    ),
    (
        "vivaldi-gpu-cache-default",
        r"Vivaldi\User Data\Default\GPUCache",
        KnownFolder::LocalAppData,
    ),
    (
        "vivaldi-cache-profiles",
        r"Vivaldi\User Data\Profile *\Cache",
        KnownFolder::LocalAppData,
    ),
    (
        "vivaldi-code-cache-profiles",
        r"Vivaldi\User Data\Profile *\Code Cache",
        KnownFolder::LocalAppData,
    ),
    (
        "vivaldi-gpu-cache-profiles",
        r"Vivaldi\User Data\Profile *\GPUCache",
        KnownFolder::LocalAppData,
    ),
    (
        "opera-cache",
        r"Opera Software\Opera *\Cache",
        KnownFolder::LocalAppData,
    ),
    (
        "opera-default-cache",
        r"Opera Software\Opera *\Default\Cache",
        KnownFolder::LocalAppData,
    ),
    (
        "opera-code-cache",
        r"Opera Software\Opera *\Default\Code Cache",
        KnownFolder::RoamingAppData,
    ),
    (
        "opera-gpu-cache",
        r"Opera Software\Opera *\Default\GPUCache",
        KnownFolder::RoamingAppData,
    ),
];

#[test]
fn vivaldi_and_opera_leaf_cache_targets_are_present() {
    let targets = embedded_targets();
    for &(id, subpath, expected_anchor) in VIVALDI_OPERA_LEAF_TARGETS {
        let t = targets
            .iter()
            .find(|t| t.id == id)
            .unwrap_or_else(|| panic!("missing {id} target"));
        let anchor = (t.known_folder, t.relative_subpath.as_str(), t.tier);
        assert_eq!(anchor, (expected_anchor, subpath, Tier::Tier2), "{id}");
        let safety = (t.risk, t.scope, t.requires_elevation, t.min_age_hours);
        assert_eq!(safety, (Risk::Safe, Scope::Contents, false, 24), "{id}");
        assert!(!t.description.trim().is_empty(), "{id}");
    }

    // No broad profile/root scoping: every Vivaldi/Opera target must end in an explicit cache leaf.
    let leaf_caches = ["cache", "code cache", "gpucache"];
    for t in targets.iter().filter(|t| {
        t.relative_subpath.contains("Vivaldi") || t.relative_subpath.contains("Opera Software")
    }) {
        let last = t.relative_subpath.rsplit('\\').next().unwrap();
        assert!(
            leaf_caches.contains(&last.to_ascii_lowercase().as_str()),
            "{} targets a non-leaf path: {}",
            t.id,
            t.relative_subpath
        );
    }
}

#[test]
fn vivaldi_and_opera_cache_kb_annotations_are_present() {
    let kb = builtin_knowledge_entries();
    let entry = kb
        .iter()
        .find(|e| e.id == "kb-vivaldi-opera-browser-caches")
        .unwrap_or_else(|| panic!("missing Vivaldi/Opera Knowledge Base annotation"));
    let text = format!("{} {} {}", entry.title, entry.description, entry.notes);
    assert_eq!(
        (entry.risk, entry.known_folder),
        (Risk::Safe, Some(KnownFolder::LocalAppData))
    );
    let documented = [
        text.contains("Vivaldi"),
        text.contains("Opera"),
        text.to_ascii_lowercase().contains("profile data"),
        entry
            .relative_pattern
            .contains(r"Opera Software\Opera *\Default\Cache"),
    ];
    assert_eq!(
        documented,
        [true, true, true, true],
        "annotation must mention Vivaldi, Opera, that profile data is not targeted, \
         and the Opera Default HTTP-cache leaf: {text}"
    );
}

#[test]
fn explorer_icon_web_cache_leaves_are_documented_out_of_scope() {
    let kb = builtin_knowledge_entries();
    let entry = kb
        .iter()
        .find(|e| e.id == "kb-explorer-icon-web-cache-leaves")
        .unwrap_or_else(|| panic!("missing Explorer icon/web cache out-of-scope annotation"));
    let text = format!("{} {}", entry.description, entry.notes).to_ascii_lowercase();
    let documented = [
        text.contains("iconcache"),
        text.contains("thumbcache"),
        text.contains("out of scope"),
        text.contains("non-cache"),
        text.contains("locked") || text.contains("in use") || text.contains("running"),
    ];
    assert_eq!(documented, [true, true, true, true, true], "{text}");

    // No broad Explorer folder target and no individual cache-DB file target.
    let targets = embedded_targets();
    let no_broad_target = targets
        .iter()
        .all(|t| t.relative_subpath != r"Microsoft\Windows\Explorer");
    let no_cache_db_file = targets.iter().all(|t| {
        let s = t.relative_subpath.to_ascii_lowercase();
        !s.contains("iconcache") && !s.contains("thumbcache_")
    });
    assert_eq!((no_broad_target, no_cache_db_file), (true, true));
}

#[test]
fn uv_and_maven_analyzer_entries_document_safe_mechanism_rationale() {
    let kb = builtin_knowledge_entries();

    let uv = kb
        .iter()
        .find(|e| e.id == "kb-uv-cache")
        .expect("missing uv Knowledge Base entry");
    let uv_text = format!("{} {}", uv.description, uv.notes).to_ascii_lowercase();
    let uv_words = uv_text.replace('-', " ");
    let uv_documented = [
        uv_words.contains("whole cache"),
        uv_text.contains("uv cache clean"),
        uv_text.contains("per-file"),
        uv_text.contains("not cleanable") || uv_text.contains("analyzer only"),
    ];
    assert_eq!(uv_documented, [true, true, true, true], "{uv_text}");

    let maven = kb
        .iter()
        .find(|e| e.id == "kb-maven-repository-cache")
        .expect("missing Maven Knowledge Base entry");
    let maven_text = format!("{} {}", maven.description, maven.notes).to_ascii_lowercase();
    let maven_words = maven_text.replace('-', " ");
    let maven_documented = [
        maven_text.contains("locally installed"),
        maven_words.contains("whole cache") || maven_text.contains("tool-driven"),
        maven_text.contains("not cleanable") || maven_text.contains("analyzer only"),
    ];
    assert_eq!(maven_documented, [true, true, true], "{maven_text}");
}
