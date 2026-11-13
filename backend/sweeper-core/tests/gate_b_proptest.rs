use proptest::prelude::*;
use proptest::string::string_regex;
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
        r"C:\".into(),
    ]
}

fn arb_segment() -> impl Strategy<Value = String> {
    prop_oneof![
        string_regex("[A-Za-z0-9_-]{1,12}").unwrap(),
        string_regex("[A-Za-z]{1,4}[0-9]{0,4}").unwrap(),
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

fn random_case(s: &str, mask: u64) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, ch) in s.chars().enumerate() {
        let bit = (mask >> (i as u32 % 63)) & 1 == 1;
        out.push(if bit {
            ch.to_ascii_uppercase()
        } else {
            ch.to_ascii_lowercase()
        });
    }
    out
}

fn mixed_separators(s: &str, mask: u64) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, ch) in s.chars().enumerate() {
        if ch == '\\' {
            let bit = (mask >> (i as u32 % 63)) & 1 == 1;
            out.push(if bit { '/' } else { '\\' });
        } else {
            out.push(ch);
        }
    }
    out
}

proptest! {
    #![proptest_config({
        let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(4096);
        ProptestConfig::with_cases(cases)
    })]

    #[test]
    fn protected_subtree_or_container_never_accepted_for_selection(
        base in prop::sample::select(protected_bases()),
        suffix in arb_child_suffix(),
        case_mask in any::<u64>(),
        sep_mask in any::<u64>(),
    ) {
        let candidate_str = format!("{}{}", base, suffix);
        let mutated = mixed_separators(&random_case(&candidate_str, case_mask), sep_mask);
        let parsed = if candidate_str == r"C:\" || mutated.eq_ignore_ascii_case("c:/") {
            parse_drive_root(&candidate_str).or_else(|_| parse_drive_root(&mutated.replace('/', "\\")))
        } else {
            CanonicalPath::parse_lexical(&mutated)
        };
        if let Ok(p) = parsed {
            if policy::is_protected_for_selection(&p) {
                prop_assert!(!policy::validate_custom_path_lexical(p.as_str()).allowed,
                    "lexical custom path accepted protected selection path: {} (from {:?})", p.as_str(), mutated);
                prop_assert!(!policy::validate_selectable_path(&p).allowed,
                    "selectable path accepted protected: {}", p.as_str());
            }
        }
    }

    #[test]
    fn adversarial_mutations_of_protected_paths_never_normalize_to_accepted(
        base in prop::sample::select(protected_bases()),
        suffix in arb_child_suffix(),
        case_mask in any::<u64>(),
        sep_mask in any::<u64>(),
        mode in 0u8..6,
    ) {
        let raw = format!("{}{}", base, suffix);
        let mut candidate = mixed_separators(&random_case(&raw, case_mask), sep_mask);
        // Apply adversarial mutation modes (some should parse-fail/reject, none should become accepted protected selection)
        candidate = match mode {
            0 => candidate, // mixed case/separators only
            1 => format!("{} ", candidate), // trailing space (likely rejected lexically)
            2 => format!("{}.", candidate), // trailing dot
            3 => format!(r"\\?\{}", candidate.replace('/', "\\")), // device prefix
            4 => format!(r"\\.\{}", candidate.replace('/', "\\")), // device prefix
            _ => format!(r"\??\{}", candidate.replace('/', "\\")), // verbatim-like prefix
        };
        // Must never be accepted as a custom path/selection target
        prop_assert!(!policy::validate_custom_path_lexical(&candidate).allowed,
            "accepted adversarial protected variant: {:?}", candidate);
        if let Ok(p) = CanonicalPath::parse_lexical(&candidate).or_else(|_| parse_drive_root(&candidate.replace('/', "\\"))) {
            if policy::is_protected_for_selection(&p) {
                prop_assert!(!policy::validate_selectable_path(&p).allowed,
                    "selectable accepted protected parsed variant: {} from {:?}", p.as_str(), candidate);
            }
        }
    }

    #[test]
    fn arbitrary_strings_never_accept_protected_when_parsed(s in "\\PC{0,100}") {
        if let Ok(p) = CanonicalPath::parse_lexical(&s).or_else(|_| parse_drive_root(&s)) {
            if policy::is_subtree_protected(&p) || policy::is_protected_container_root(&p) {
                prop_assert!(!policy::validate_custom_path_lexical(p.as_str()).allowed,
                    "accepted protected parsed path: {} from {:?}", p.as_str(), s);
            }
        } else {
            prop_assert!(!policy::validate_custom_path_lexical(&s).allowed);
        }
    }

    #[test]
    fn segment_boundary_prefix_property(a in string_regex("[A-Za-z]{3,8}").unwrap(), b in string_regex("[A-Za-z]{1,4}").unwrap()) {
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
