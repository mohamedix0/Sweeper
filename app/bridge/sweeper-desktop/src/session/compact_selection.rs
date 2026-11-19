//! Compact selection decoding for `preview_clean`.
//!
//! The IPC stays ID-only (ADR 0002) and the command signature is unchanged: a
//! large selection is sent as one sentinel-prefixed descriptor that expands to
//! the exact explicit item IDs before the existing session validation runs.
//! Malformed or oversized descriptors are returned unchanged so the existing
//! `unknownItem` validation rejects them (fail closed).

use serde::Deserialize;

const SENTINEL: &str = "SWSELECT1:";
const MAX_COMPACT_IDS: usize = 1_000_000;

#[derive(Deserialize)]
struct Range {
    prefix: String,
    start: u64,
    end: u64,
}

#[derive(Deserialize)]
struct Descriptor {
    ranges: Vec<Range>,
    explicit: Vec<String>,
}

/// Decode a single sentinel-prefixed wire value; `None` means "not our descriptor".
fn decode_descriptor(item_ids: &[String]) -> Option<Descriptor> {
    if item_ids.len() != 1 {
        return None;
    }
    let body = item_ids[0].strip_prefix(SENTINEL)?;
    serde_json::from_str::<Descriptor>(body).ok()
}

/// Number of IDs in a range, or `None` when its bounds are corrupt or unsafe.
fn range_span(range: &Range) -> Option<usize> {
    if range.start == 0 {
        return None;
    }
    if range.end < range.start {
        return None;
    }
    Some(usize::try_from(range.end - range.start + 1).unwrap_or(usize::MAX))
}

/// Expand all ranges into explicit IDs; `None` if any range is invalid or oversized.
fn expand_ranges(ranges: &[Range]) -> Option<Vec<String>> {
    let mut expanded: Vec<String> = Vec::new();
    for range in ranges {
        let span = range_span(range)?;
        if span > MAX_COMPACT_IDS.saturating_sub(expanded.len()) {
            return None;
        }
        for seq in range.start..=range.end {
            expanded.push(format!("{}::{}", range.prefix, seq));
        }
    }
    Some(expanded)
}

/// Append verbatim explicit IDs; `false` once the global cap is reached.
fn append_explicit(expanded: &mut Vec<String>, explicit: &[String]) -> bool {
    for id in explicit {
        if expanded.len() >= MAX_COMPACT_IDS {
            return false;
        }
        expanded.push(id.clone());
    }
    true
}

pub(crate) fn expand_selection_ids(item_ids: &[String]) -> Vec<String> {
    let Some(descriptor) = decode_descriptor(item_ids) else {
        return item_ids.to_vec();
    };
    let Some(mut expanded) = expand_ranges(&descriptor.ranges) else {
        return item_ids.to_vec();
    };
    if !append_explicit(&mut expanded, &descriptor.explicit) {
        return item_ids.to_vec();
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::{expand_selection_ids, SENTINEL};

    fn id(value: &str) -> String {
        value.to_string()
    }

    #[test]
    fn explicit_ids_are_unchanged() {
        let input = vec![id("user-temp::1"), id("user-temp::2")];
        assert_eq!(expand_selection_ids(&input), input);
    }

    #[test]
    fn compact_descriptor_expands_to_exact_ids() {
        let wire = vec![id(&format!(
            "{SENTINEL}{{\"ranges\":[{{\"prefix\":\"user-temp\",\"start\":1,\"end\":3}}],\"explicit\":[\"recycle-bin\"]}}"
        ))];
        assert_eq!(
            expand_selection_ids(&wire),
            vec![
                id("user-temp::1"),
                id("user-temp::2"),
                id("user-temp::3"),
                id("recycle-bin")
            ]
        );
    }

    #[test]
    fn malformed_descriptor_fails_closed() {
        let wire = vec![id(&format!("{SENTINEL}not-json"))];
        assert_eq!(expand_selection_ids(&wire), wire);
    }

    #[test]
    fn oversized_descriptor_fails_closed() {
        let wire = vec![id(&format!(
            "{SENTINEL}{{\"ranges\":[{{\"prefix\":\"user-temp\",\"start\":1,\"end\":2000000}}],\"explicit\":[]}}"
        ))];
        assert_eq!(expand_selection_ids(&wire), wire);
    }
}
