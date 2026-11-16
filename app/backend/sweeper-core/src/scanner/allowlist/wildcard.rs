use super::*;
use std::path::{Path, PathBuf};

pub(in crate::scanner) fn validate_and_collect_roots_for_target(
    target: &TargetDefinition,
    known_folder_root: &CanonicalPath,
    override_root: Option<&CanonicalPath>,
) -> Result<Vec<(String, CanonicalPath)>, String> {
    if let Some(roots) = redirected_temp_override_roots(target, override_root)? {
        return Ok(roots);
    }
    let candidates = expand_target_candidates(known_folder_root, &target.relative_subpath);
    collect_validated_candidate_roots(target, candidates)
}

fn redirected_temp_override_roots(
    target: &TargetDefinition,
    override_root: Option<&CanonicalPath>,
) -> Result<Option<Vec<(String, CanonicalPath)>>, String> {
    if target.id != "redirected-temp" {
        return Ok(None);
    }
    let Some(path) = override_root else {
        return Ok(None);
    };
    let root = validate_single_resolved_root(path.clone())?;
    Ok(Some(vec![("redirected-temp".to_string(), root)]))
}

fn expand_target_candidates(known_folder_root: &CanonicalPath, rel: &str) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(known_folder_root.as_str())];
    for seg in rel.split('\\') {
        candidates = expand_segment_candidates(candidates, seg);
    }
    candidates
}

fn expand_segment_candidates(candidates: Vec<PathBuf>, seg: &str) -> Vec<PathBuf> {
    if seg.contains('*') {
        return expand_wildcard_segment(candidates, seg);
    }
    append_literal_segment(candidates, seg)
}

fn append_literal_segment(mut candidates: Vec<PathBuf>, seg: &str) -> Vec<PathBuf> {
    for c in &mut candidates {
        c.push(seg);
    }
    candidates
}

fn collect_validated_candidate_roots(
    target: &TargetDefinition,
    candidates: Vec<PathBuf>,
) -> Result<Vec<(String, CanonicalPath)>, String> {
    let mut roots = Vec::new();
    for c in candidates {
        if !c.exists() {
            continue;
        }
        let p = validate_resolved_root_path(&c)
            .map_err(|e| format!("resolved target root invalid: {e}"))?;
        roots.push((target.id.clone(), p));
    }
    dedupe_roots(&mut roots);
    Ok(roots)
}

fn dedupe_roots(roots: &mut Vec<(String, CanonicalPath)>) {
    roots.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    roots.dedup_by(|a, b| a.1.is_same_path(&b.1));
}

pub(super) fn expand_wildcard_segment(candidates: Vec<PathBuf>, seg: &str) -> Vec<PathBuf> {
    let mut next = Vec::new();
    for cand in candidates {
        expand_wildcard_candidate(&cand, seg, &mut next);
    }
    next
}

fn expand_wildcard_candidate(cand: &Path, seg: &str, next: &mut Vec<PathBuf>) {
    if !cand.exists() {
        return;
    }
    let entries = match sweeper_winfs::read_dir_entries(cand) {
        Ok(v) => v,
        Err(_) => return,
    };
    for ent in entries {
        if is_eligible_wildcard_entry(&ent, seg) {
            next.push(ent.path);
        }
    }
}

fn is_eligible_wildcard_entry(ent: &sweeper_winfs::DirEntry, seg: &str) -> bool {
    if !is_wildcard_name_match(ent, seg) {
        return false;
    }
    if is_reparse_wildcard_entry(ent) {
        return false;
    }
    is_non_link_point_directory(ent)
}

fn is_wildcard_name_match(ent: &sweeper_winfs::DirEntry, seg: &str) -> bool {
    if !ent.is_dir {
        return false;
    }
    super::policy::segment_pattern_matches(&ent.name.to_string_lossy(), seg)
}

fn is_reparse_wildcard_entry(ent: &sweeper_winfs::DirEntry) -> bool {
    !matches!(ent.reparse, ReparseTagClass::NotReparse)
}

fn is_non_link_point_directory(ent: &sweeper_winfs::DirEntry) -> bool {
    !sweeper_winfs::open_for_inspection(&ent.path)
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(true)
}

fn validate_single_resolved_root(path: CanonicalPath) -> Result<CanonicalPath, String> {
    super::policy::validate_resolved_canonical_root(&path).map(|_| path)
}

pub(super) fn validate_resolved_root_path(path: &Path) -> Result<CanonicalPath, String> {
    let validated =
        CanonicalPath::from_existing_path(&path.to_string_lossy()).map_err(|e| e.to_string())?;
    let canon = validated.into_canonical();
    super::policy::validate_resolved_canonical_root(&canon)?;
    Ok(canon)
}
