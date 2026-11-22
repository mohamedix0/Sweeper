//! Case-variant sibling detection for moment-of-deletion revalidation.
//!
//! The folded path comparison in `super::revalidate_handle` cannot distinguish
//! `sub` from `SUB` when the parent directory treats names case-sensitively.
//! This module inspects the exact on-disk entry names at the point where the
//! confirmed and handle-resolved paths diverge, using read-only enumeration
//! that never traverses entries.

use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::paths::CanonicalPath;

/// Exact comparison except for the drive letter. `is_same_path` folds case,
/// which is correct for ordinary paths but hides a case-variant sibling on a
/// directory with per-directory case sensitivity enabled.
fn same_path_exact_except_drive(a: &CanonicalPath, b: &CanonicalPath) -> bool {
    if !a.drive_prefix().eq_ignore_ascii_case(b.drive_prefix()) {
        return false;
    }
    a.as_str()[2..] == b.as_str()[2..]
}

/// Index of the first segment whose exact on-disk spelling differs.
fn first_case_divergent_segment(a: &CanonicalPath, b: &CanonicalPath) -> Option<usize> {
    a.segments().zip(b.segments()).position(|(x, y)| x != y)
}

/// Directory containing the divergent component: the drive root plus the
/// shared prefix segments above the first difference.
fn case_variant_ancestor(path: &CanonicalPath, divergent_index: usize) -> String {
    let shared: Vec<&str> = path.segments().take(divergent_index).collect();
    if shared.is_empty() {
        format!("{}\\", path.drive_prefix())
    } else {
        format!("{}\\{}", path.drive_prefix(), shared.join("\\"))
    }
}

/// True only when both exact spellings appear as distinct directory entries.
/// On a case-insensitive directory only the real spelling is listed, so a
/// benign difference in case spelling is accepted.
fn exact_names_both_present(names: &[OsString], requested: &str, final_name: &str) -> bool {
    let mut requested_seen = false;
    let mut final_seen = false;
    for name in names {
        if name == OsStr::new(requested) {
            requested_seen = true;
        }
        if name == OsStr::new(final_name) {
            final_seen = true;
        }
    }
    requested_seen && final_seen
}

/// `read_dir_entries` reports exact on-disk names without following entries.
/// An enumeration failure aborts: denial is safer than an unchecked pass.
pub(super) fn reject_case_distinct_sibling(
    final_canon: &CanonicalPath,
    requested_canon: &CanonicalPath,
) -> Result<(), String> {
    reject_case_distinct_sibling_with(final_canon, requested_canon, |ancestor| {
        sweeper_winfs::read_dir_entries(ancestor)
            .map(|entries| entries.into_iter().map(|entry| entry.name).collect())
            .map_err(|e| e.to_string())
    })
}

/// Core of the case-variant check with enumeration injected so it is testable
/// without elevation or a case-sensitive volume.
fn reject_case_distinct_sibling_with<F>(
    final_canon: &CanonicalPath,
    requested_canon: &CanonicalPath,
    list_names: F,
) -> Result<(), String>
where
    F: FnOnce(&Path) -> Result<Vec<OsString>, String>,
{
    if same_path_exact_except_drive(final_canon, requested_canon) {
        return Ok(());
    }
    let Some(index) = first_case_divergent_segment(final_canon, requested_canon) else {
        return Ok(());
    };
    let ancestor = case_variant_ancestor(requested_canon, index);
    let requested_name = requested_canon.segments().nth(index).unwrap_or("");
    let final_name = final_canon.segments().nth(index).unwrap_or("");
    let names = list_names(Path::new(&ancestor)).map_err(|e| {
        format!("aborted: escape via case-variant ancestor enumeration failure: {e}")
    })?;
    if exact_names_both_present(&names, requested_name, final_name) {
        return Err(
            "aborted: escape via case-distinct sibling swap (link point to case-variant directory)"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_distinct_sibling_in_listing_aborts() {
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\sub\victim.txt").unwrap();
        let names = vec![
            OsString::from("sub"),
            OsString::from("SUB"),
            OsString::from("victim.txt"),
        ];
        let reason =
            reject_case_distinct_sibling_with(
                &final_canon,
                &requested_canon,
                |_| Ok(names.clone()),
            )
            .expect_err("two exact spellings in one directory must abort");
        // The sole caller downgrades a reason to Skipped unless it contains
        // "escape" (or both "target root" and "link point"); prove it cannot.
        assert!(reason.contains("aborted"), "{reason}");
        assert!(reason.contains("escape"), "{reason}");
    }

    #[test]
    fn benign_case_spelling_of_same_entry_is_accepted() {
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\sub\victim.txt").unwrap();
        // A case-insensitive directory lists only the real on-disk spelling.
        let names = vec![OsString::from("sub"), OsString::from("victim.txt")];
        assert!(
            reject_case_distinct_sibling_with(&final_canon, &requested_canon, |_| {
                Ok(names.clone())
            })
            .is_ok()
        );
    }

    #[test]
    fn case_variant_enumeration_failure_aborts() {
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\sub\victim.txt").unwrap();
        let reason = reject_case_distinct_sibling_with(&final_canon, &requested_canon, |_| {
            Err("access denied".to_string())
        })
        .expect_err("enumeration failure must be denied by default");
        assert!(reason.contains("aborted"), "{reason}");
        assert!(reason.contains("escape"), "{reason}");
    }
}
