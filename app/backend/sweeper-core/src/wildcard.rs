//! Shared allowlist segment wildcard matching.
//!
//! Both the scanner's allowlist expansion and the Analyzer's annotation context
//! resolve target subpaths that may contain a single `*` wildcard per segment.
//! Keeping the matcher here gives them one implementation so the two callers
//! cannot drift apart.
//!
//! Edge comparisons operate on byte slices rather than `&str` slices: a
//! multi-byte UTF-8 directory name can satisfy a byte-length guard while the
//! prefix/suffix boundary falls in the middle of a character, which would make
//! a `&str` slice panic.

/// Match a concrete path segment `name` against an allowlist `pattern` that may
/// contain a single `*` wildcard (case-insensitive for ASCII).
///
/// Semantics: no `*`/one `*` at either or both edges; patterns with more than
/// one `*` are rejected (callers validate/lint those separately).
pub(crate) fn segment_pattern_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return name.eq_ignore_ascii_case(pattern);
    }
    // A single '*' wildcard splits the pattern into a prefix and suffix.
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return false;
    }
    let (prefix, suffix) = (parts[0], parts[1]);
    let name_bytes = name.as_bytes();
    let pre_len = prefix.len();
    let suf_len = suffix.len();
    if name_bytes.len() < pre_len.saturating_add(suf_len) {
        return false;
    }
    // Byte-slice comparisons are always panic-free: no character-boundary
    // invariant is required, unlike `name[..pre_len]` / `name[len - suf_len..]`.
    name_bytes[..pre_len].eq_ignore_ascii_case(prefix.as_bytes())
        && name_bytes[name_bytes.len() - suf_len..].eq_ignore_ascii_case(suffix.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::segment_pattern_matches;

    #[test]
    fn exact_segment_match_is_ascii_case_insensitive() {
        assert!(segment_pattern_matches("Default", "default"));
        assert!(segment_pattern_matches("PROFILE 1", "profile 1"));
        assert!(!segment_pattern_matches("Default", "Profile 1"));
    }

    #[test]
    fn prefix_suffix_and_middle_wildcards_match_ascii_names() {
        assert!(segment_pattern_matches("Profile 1", "Profile *"));
        assert!(segment_pattern_matches("PROFILE 42", "profile *"));
        assert!(segment_pattern_matches("Code Cache", "*cache"));
        assert!(segment_pattern_matches("CODE CACHE", "*Cache"));
        assert!(segment_pattern_matches("PreMIDDLEsuf", "Pre*Suf"));
        assert!(segment_pattern_matches("premiddlesuf", "PRE*suf"));
    }

    #[test]
    fn multi_byte_name_with_valid_byte_length_does_not_panic_on_prefix_wildcard() {
        // F5: "aéééé" is 9 bytes; prefix "Profile " is 8 bytes. The old guard
        // (9 >= 8) passed, then name[..8] split the final 'é' mid-character.
        assert!(!segment_pattern_matches(
            "a\u{e9}\u{e9}\u{e9}\u{e9}",
            "Profile *"
        ));
        // Still matches when the ASCII prefix really is present.
        assert!(segment_pattern_matches(
            "Profile \u{e9}\u{e9}\u{e9}",
            "PROFILE *"
        ));
    }

    #[test]
    fn multi_byte_name_with_valid_byte_length_does_not_panic_on_suffix_wildcard() {
        // "ééé" is 6 bytes; suffix "abc" is 3 bytes, so the old guard passed
        // and name[6 - 3..] started in the middle of the second 'é'.
        assert!(!segment_pattern_matches("\u{e9}\u{e9}\u{e9}", "*abc"));
        assert!(segment_pattern_matches("\u{e9}\u{e9}Cache", "*cache"));
    }

    #[test]
    fn multi_byte_name_with_valid_byte_length_does_not_panic_on_middle_wildcard() {
        // "Preéé" is 7 bytes; suffix "Suf" is 3 bytes, so the old guard passed
        // and name[7 - 3..] started in the middle of the first 'é'.
        assert!(!segment_pattern_matches("Pre\u{e9}\u{e9}", "Pre*Suf"));
        assert!(segment_pattern_matches("Pre\u{e9}\u{e9}Suf", "pre*suf"));
    }

    #[test]
    fn short_name_and_ambiguous_pattern_are_rejected() {
        assert!(!segment_pattern_matches("abc", "abcd*ef"));
        assert!(!segment_pattern_matches("a*b*c", "a*b*c"));
        assert!(segment_pattern_matches("anything", "*"));
    }
}
