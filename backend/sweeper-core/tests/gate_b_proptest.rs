use proptest::prelude::*;
use sweeper_core::paths::{parse_drive_root, CanonicalPath};
use sweeper_core::policy;

fn protected_bases() -> Vec<String> {
    vec![
        r"C:\Windows\System32".into(),
        r"C:\Windows\WinSxS".into(),
        r"C:\Windows\SysWOW64".into(),
        r"C:\Windows\Boot".into(),
        r"C:\Windows\Fonts".into(),
        r"C:\Windows\servicing".into(),
        r"C:\Program Files".into(),
        r"C:\Program Files (x86)".into(),
        r"C:\Recovery".into(),
        r"C:\Windows".into(),
        r"C:\Users".into(),
        r"C:\ProgramData".into(),
    ]
}

fn arb_segment() -> impl Strategy<Value = String> {
    // Mix of normal, case, unicode, tricky but mostly parseable child names
    prop_oneof![
        "[A-Za-z0-9_-]{1,12}".prop_map(|s| s),
        "[A-Za-z]{1,4}[0-9]{0,4}".prop_map(|s| s),
        Just("Temp".to_string()),
        Just("Cache".to_string()),
        Just("Data".to_string()),
        Just("sub.dir".to_string()),
        Just("مرحبا".to_string()),
        Just("emoji😀".to_string()),
    ]
}

fn arb_child_suffix() -> impl Strategy<Value = String> {
    prop::collection::vec(arb_segment(), 0..4).prop_map(|v| {
        if v.is_empty() {
            String::new()
        } else {
            format!("\\{}", v.join("\\"))
        }
    })
}

proptest! {
    #![proptest_config({
        let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256);
        ProptestConfig::with_cases(cases)
    })]

    #[test]
    fn protected_subtree_or_container_never_accepted_for_selection(base in prop::sample::select(protected_bases()), suffix in arb_child_suffix()) {
        let candidate_str = format!("{}{}", base, suffix);
        let parsed = if candidate_str == r"C:\" { parse_drive_root(&candidate_str) } else { CanonicalPath::parse_lexical(&candidate_str) };
        if let Ok(p) = parsed {
            // if it's under/equals subtree-protected or is container root/protected-for-selection, must reject
            if policy::is_protected_for_selection(&p) {
                prop_assert!(!policy::validate_custom_path_lexical(p.as_str()).allowed,
                    "lexical custom path accepted protected selection path: {}", p.as_str());
                let verdict = policy::validate_selectable_path(&p);
                prop_assert!(!verdict.allowed, "selectable path accepted protected: {}", p.as_str());
            }
        }
    }

    #[test]
    fn arbitrary_strings_never_accept_protected_when_parsed(s in "\\PC{0,80}") {
        if let Ok(p) = CanonicalPath::parse_lexical(&s).or_else(|_| parse_drive_root(&s)) {
            if policy::is_subtree_protected(&p) || policy::is_protected_container_root(&p) {
                prop_assert!(!policy::validate_custom_path_lexical(p.as_str()).allowed,
                    "accepted protected parsed path: {} from {:?}", p.as_str(), s);
            }
        } else {
            // unparseable must not be accepted
            prop_assert!(!policy::validate_custom_path_lexical(&s).allowed);
        }
    }

    #[test]
    fn segment_boundary_prefix_property(a in "[A-Za-z]{3,8}", b in "[A-Za-z]{1,4}") {
        // C:\<a> vs C:\<a><b> must not be treated as ancestor unless segment boundary
        let base_s = format!(r"C:\{}", a);
        let child_same_prefix = format!(r"C:\{}{}\foo\bar", a, b);
        let real_child = format!(r"C:\{}\{}\bar", a, b);
        if let (Ok(base), Ok(fake), Ok(real)) = (
            CanonicalPath::parse_lexical(&base_s),
            CanonicalPath::parse_lexical(&child_same_prefix),
            CanonicalPath::parse_lexical(&real_child),
        ) {
            prop_assert!(!base.is_self_or_ancestor_of(&fake), "{} should not be ancestor of {}", base_s, child_same_prefix);
            prop_assert!(base.is_self_or_ancestor_of(&real), "{} should be ancestor of {}", base_s, real_child);
        }
    }
}
