//! Case-variant sibling detection for moment-of-deletion revalidation.
//!
//! The folded path comparison in `super::revalidate_handle` cannot distinguish
//! `sub` from `SUB` when the parent directory treats names case-sensitively.
//! This module compares the on-disk identity of BOTH spellings at EVERY segment
//! where the confirmed and handle-resolved paths differ. Identity (not a single
//! directory listing) proves that the two spellings name the same entry, even
//! if a junction is removed between the open and this check.

use std::path::Path;

use sweeper_winfs::FileIdentity;

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

/// Directory containing a divergent component: the drive root plus the shared
/// prefix segments above that index.
fn case_variant_ancestor(path: &CanonicalPath, divergent_index: usize) -> String {
    let shared: Vec<&str> = path.segments().take(divergent_index).collect();
    if shared.is_empty() {
        format!("{}\\", path.drive_prefix())
    } else {
        format!("{}\\{}", path.drive_prefix(), shared.join("\\"))
    }
}

/// Production identity provider: open without following reparse points, then
/// read the handle's volume serial + file index. A junction keeps its own
/// identity, so it cannot masquerade as its target.
pub(super) fn reject_case_distinct_sibling(
    final_canon: &CanonicalPath,
    requested_canon: &CanonicalPath,
) -> Result<(), String> {
    reject_case_distinct_sibling_with(final_canon, requested_canon, |path| {
        let handle = sweeper_winfs::open_for_inspection(path).map_err(|e| e.to_string())?;
        sweeper_winfs::get_file_identity(&handle).map_err(|e| e.to_string())
    })
}

/// Compare every differing segment with an injectable identity provider so the
/// check is testable without elevation or a case-sensitive volume.
fn reject_case_distinct_sibling_with<F>(
    final_canon: &CanonicalPath,
    requested_canon: &CanonicalPath,
    mut identity_of: F,
) -> Result<(), String>
where
    F: FnMut(&Path) -> Result<FileIdentity, String>,
{
    if same_path_exact_except_drive(final_canon, requested_canon) {
        return Ok(());
    }
    let requested_segments: Vec<&str> = requested_canon.segments().collect();
    let final_segments: Vec<&str> = final_canon.segments().collect();
    if requested_segments.len() != final_segments.len() {
        return Err("aborted: escape via case-variant segment count divergence".to_string());
    }
    for index in 0..requested_segments.len() {
        if requested_segments[index] == final_segments[index] {
            continue;
        }
        check_case_variant_segment(
            requested_canon,
            index,
            requested_segments[index],
            final_segments[index],
            &mut identity_of,
        )?;
    }
    Ok(())
}

/// Both spellings must open AND resolve to the same identity; any failure is
/// denied by default because a missing spelling is not proof of identity.
fn check_case_variant_segment<F>(
    requested_canon: &CanonicalPath,
    index: usize,
    requested_name: &str,
    final_name: &str,
    identity_of: &mut F,
) -> Result<(), String>
where
    F: FnMut(&Path) -> Result<FileIdentity, String>,
{
    let parent = case_variant_ancestor(requested_canon, index);
    let parent = Path::new(&parent);
    let requested_identity = identity_of(&parent.join(requested_name)).map_err(|e| {
        format!("aborted: escape via case-variant requested entry open failure: {e}")
    })?;
    let final_identity = identity_of(&parent.join(final_name))
        .map_err(|e| format!("aborted: escape via case-variant final entry open failure: {e}"))?;
    if requested_identity != final_identity {
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

    fn identity(index: u64) -> FileIdentity {
        FileIdentity {
            volume_serial: 1,
            file_index: index,
        }
    }

    fn identity_of_well_known_names(path: &Path) -> Result<FileIdentity, String> {
        match path.file_name().and_then(|name| name.to_str()) {
            Some("temp") | Some("Temp") => Ok(identity(10)),
            Some("sub") => Ok(identity(20)),
            Some("SUB") => Ok(identity(21)),
            Some("victim.txt") => Ok(identity(30)),
            _ => Err("unexpected path".to_string()),
        }
    }

    #[test]
    fn later_case_variant_segment_with_distinct_identity_aborts() {
        // Finding A: the first difference (`temp`/`Temp`) is benign, the later
        // `sub`/`SUB` pair is a different entry.
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\temp\d1\sub\victim.txt").unwrap();
        let reason = reject_case_distinct_sibling_with(
            &final_canon,
            &requested_canon,
            identity_of_well_known_names,
        )
        .expect_err("a later case-distinct sibling must abort");
        assert!(reason.contains("escape"), "{reason}");
    }

    #[test]
    fn missing_requested_spelling_aborts() {
        // Finding B: the junction at `sub` is gone by the time of the check;
        // one surviving spelling is not proof that the handle is unchanged.
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\temp\d1\sub\victim.txt").unwrap();
        let reason =
            reject_case_distinct_sibling_with(&final_canon, &requested_canon, |path: &Path| {
                if path.file_name().and_then(|n| n.to_str()) == Some("sub") {
                    return Err("not found".to_string());
                }
                identity_of_well_known_names(path)
            })
            .expect_err("a missing requested spelling must abort");
        assert!(reason.contains("escape"), "{reason}");
    }

    #[test]
    fn identical_identities_accepted() {
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\temp\d1\sub\victim.txt").unwrap();
        assert!(reject_case_distinct_sibling_with(
            &final_canon,
            &requested_canon,
            |path: &Path| {
                let _ = path;
                Ok(identity(7))
            },
        )
        .is_ok());
    }

    #[test]
    fn identity_query_failure_aborts() {
        let final_canon = CanonicalPath::parse_lexical(r"C:\Temp\d1\SUB\victim.txt").unwrap();
        let requested_canon = CanonicalPath::parse_lexical(r"C:\temp\d1\sub\victim.txt").unwrap();
        let reason = reject_case_distinct_sibling_with(&final_canon, &requested_canon, |_| {
            Err("access denied".to_string())
        })
        .expect_err("identity query failure must be denied by default");
        assert!(reason.contains("escape"), "{reason}");
    }
}
