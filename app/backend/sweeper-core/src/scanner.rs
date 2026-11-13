//! Read-only scan engine for allowlisted targets (Tier 1 & Tier 2).
//!
//! Safety: no deletion logic, no `remover`, no `execute_clean`. Uses handle-aware enumeration
//! via `sweeper-winfs` and never follows `Link Point`s, never hydrates `CloudPlaceholder`s.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::{KnownFolder, Risk, TargetDefinition, Tier};
use sweeper_winfs::{self, DirEntry, FileIdentity, KnownFolderId, ReparseTagClass, WinfsError};

use crate::paths::CanonicalPath;
use crate::policy;

/// Reasons an entry is skipped during scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SkipReason {
    /// Symlink or junction (`Link Point`).
    LinkPoint,
    /// Cloud placeholder (e.g. OneDrive/cldflt). Never hydrated.
    CloudPlaceholder,
    /// Other reparse tag.
    OtherReparse,
    /// Unreadable/inaccessible entry or metadata.
    Unreadable,
}

impl SkipReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::LinkPoint => "LinkPoint",
            Self::CloudPlaceholder => "CloudPlaceholder",
            Self::OtherReparse => "OtherReparse",
            Self::Unreadable => "Unreadable",
        }
    }
}

/// Result of redirected `%TEMP%` guard evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TempGuard {
    /// Use the provided canonical redirected temp path.
    Use(CanonicalPath),
    /// Redirected temp must be auto-disabled for safety/redundancy.
    Disabled(String),
}

/// Metadata for a scannable/observed item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanItem {
    /// Stable per-scan item identifier.
    pub id: String,
    /// Owning target id.
    pub target_id: String,
    /// Canonical/resolved path string (display/internal; never submitted for deletion IPC in this crate).
    pub path: String,
    /// File identity (volume serial + file index) when available.
    pub file_identity: Option<FileIdentity>,
    /// Logical size in bytes.
    pub size: u64,
    /// Last modification time (system time seconds since UNIX epoch, saturating).
    pub mtime_unix_secs: u64,
    /// Whether the item is eligible by minimum-age policy (and not future-dated).
    pub age_eligible: bool,
}

/// Per-target scan totals and status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetScanResult {
    /// Target definition id.
    pub target_id: String,
    /// Target name.
    pub target_name: String,
    /// Tier.
    pub tier: Tier,
    /// Risk.
    pub risk: Risk,
    /// Resolved concrete root paths that were scanned (expanded from wildcards).
    pub resolved_paths: Vec<String>,
    /// Whether the target was scanned.
    pub scanned: bool,
    /// Disabled/error/status reason when not scanned or partially disabled.
    pub status_reason: Option<String>,
    /// Total bytes observed.
    pub total_bytes: u64,
    /// Eligible bytes observed.
    pub eligible_bytes: u64,
    /// Total file/item count observed.
    pub file_count: u64,
    /// Eligible file/item count observed.
    pub eligible_count: u64,
    /// Skipped entry count.
    pub skipped_count: u64,
    /// Skipped reason counts.
    pub skipped_reasons: BTreeMap<String, u64>,
    /// Detailed skip diagnostics (e.g. unreadable path/error context) for Gate E/reporting.
    #[serde(default)]
    pub skipped_diagnostics: Vec<String>,
    /// Per-item metadata.
    pub items: Vec<ScanItem>,
}

/// Aggregate scan report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanReport {
    /// When the scan started (unix seconds).
    pub started_unix_secs: u64,
    /// Minimum age in hours applied.
    pub min_age_hours: u32,
    /// Per-target results.
    pub targets: Vec<TargetScanResult>,
}

/// Scan-time errors that are not per-target disable reasons.
#[derive(Debug, Error)]
pub enum ScanError {
    /// Failed to enumerate embedded allowlist.
    #[error("failed to load embedded targets: {0}")]
    Targets(String),
}

/// Scan enabled Tier 1 & Tier 2 embedded targets with the given minimum-age override (hours).
pub fn scan_tier1_tier2(min_age_hours: u32) -> Result<ScanReport, ScanError> {
    let targets = sweeper_knowledge::embedded_targets();
    scan_targets(
        targets
            .into_iter()
            .filter(|t| matches!(t.tier, Tier::Tier1 | Tier::Tier2))
            .collect(),
        min_age_hours,
    )
}

/// Scan a provided set of targets (read-only).
pub fn scan_targets(
    targets: Vec<TargetDefinition>,
    min_age_hours: u32,
) -> Result<ScanReport, ScanError> {
    scan_targets_with_redirected_temp_override(targets, min_age_hours, None)
}

/// Scan a provided set of targets (read-only) with an optional injected redirected `%TEMP%`
/// candidate path for deterministic/testing scenarios.
///
/// When `redirected_temp_candidate` is `Some`, it is used instead of reading process `%TEMP%`
/// for `redirected-temp` guard evaluation.
pub fn scan_targets_with_redirected_temp_override(
    targets: Vec<TargetDefinition>,
    min_age_hours: u32,
    redirected_temp_candidate: Option<&str>,
) -> Result<ScanReport, ScanError> {
    validate_target_definitions(&targets).map_err(ScanError::Targets)?;
    let now = SystemTime::now();
    let started_unix_secs = system_time_to_unix_secs(now);
    let min_age = Duration::from_secs(u64::from(min_age_hours) * 3600);

    // Resolve known folder roots used by these targets.
    let needed_kfs: HashSet<KnownFolder> = targets.iter().map(|t| t.known_folder).collect();
    let mut kf_roots: BTreeMap<KnownFolder, Result<CanonicalPath, String>> = BTreeMap::new();
    for kf in needed_kfs {
        kf_roots.insert(kf, resolve_validated_known_folder_root(kf));
    }

    let mut results = Vec::with_capacity(targets.len());
    let mut all_resolved_roots: Vec<(String, CanonicalPath)> = Vec::new();

    for target in targets {
        let kf_res = kf_roots
            .get(&target.known_folder)
            .cloned()
            .unwrap_or_else(|| Err("known folder not resolved".to_string()));
        let mut out = TargetScanResult {
            target_id: target.id.clone(),
            target_name: target.name.clone(),
            tier: target.tier,
            risk: target.risk,
            resolved_paths: Vec::new(),
            scanned: false,
            status_reason: None,
            total_bytes: 0,
            eligible_bytes: 0,
            file_count: 0,
            eligible_count: 0,
            skipped_count: 0,
            skipped_reasons: BTreeMap::new(),
            skipped_diagnostics: Vec::new(),
            items: Vec::new(),
        };

        let base_root = match kf_res {
            Ok(p) => p,
            Err(e) => {
                out.status_reason = Some(format!("known folder resolution failed: {e}"));
                results.push(out);
                continue;
            }
        };

        // Special-case: redirected-temp conditional target.
        if target.id == "redirected-temp" {
            let temp_guard = match redirected_temp_candidate {
                Some(candidate) => {
                    let user_temp = match join_and_canonicalize(&base_root, "Temp") {
                        Ok(p) => p,
                        Err(e) => {
                            out.status_reason = Some(format!(
                                "redirected-temp disabled: failed to resolve user-temp: {e}"
                            ));
                            results.push(out);
                            continue;
                        }
                    };
                    evaluate_redirected_temp_candidate(Some(candidate), &user_temp)
                }
                None => evaluate_redirected_temp(&base_root),
            };
            match temp_guard {
                TempGuard::Use(path) => {
                    // scanned below using resolved path override
                    match validate_and_collect_roots_for_target(&target, &base_root, Some(&path)) {
                        Ok(roots) => {
                            if let Err(reason) =
                                validate_target_roots_policy(&roots, &all_resolved_roots)
                            {
                                out.status_reason = Some(reason);
                                results.push(out);
                                continue;
                            }
                            for (rid, r) in &roots {
                                all_resolved_roots.push((rid.clone(), r.clone()));
                            }
                            out.resolved_paths = roots.iter().map(|(_, p)| p.to_string()).collect();
                            scan_resolved_roots(&target, &roots, min_age, now, &mut out);
                            results.push(out);
                        }
                        Err(e) => {
                            out.status_reason = Some(e);
                            results.push(out);
                        }
                    }
                }
                TempGuard::Disabled(reason) => {
                    out.status_reason = Some(format!("redirected-temp disabled: {reason}"));
                    results.push(out);
                }
            }
            continue;
        }

        match validate_and_collect_roots_for_target(&target, &base_root, None) {
            Ok(roots) => {
                // Missing/unresolvable concrete roots (e.g. cache folders not present) are not scanned.
                if roots.is_empty() {
                    out.status_reason = Some("not present".to_string());
                    results.push(out);
                    continue;
                }
                if let Err(reason) = validate_target_roots_policy(&roots, &all_resolved_roots) {
                    out.status_reason = Some(reason);
                    results.push(out);
                    continue;
                }
                for (rid, r) in &roots {
                    all_resolved_roots.push((rid.clone(), r.clone()));
                }
                out.resolved_paths = roots.iter().map(|(_, p)| p.to_string()).collect();
                scan_resolved_roots(&target, &roots, min_age, now, &mut out);
                results.push(out);
            }
            Err(e) => {
                out.status_reason = Some(e);
                results.push(out);
            }
        }
    }

    Ok(ScanReport {
        started_unix_secs,
        min_age_hours,
        targets: results,
    })
}

fn scan_resolved_roots(
    target: &TargetDefinition,
    roots: &[(String, CanonicalPath)],
    min_age: Duration,
    now: SystemTime,
    out: &mut TargetScanResult,
) {
    out.scanned = true;
    let mut next_item_seq: u64 = 0;
    for (_root_id, root) in roots {
        scan_one_root_iterative(target, root, min_age, now, out, &mut next_item_seq);
    }
}

struct ScanWalkCtx<'a> {
    target: &'a TargetDefinition,
    min_age: Duration,
    now: SystemTime,
    out: &'a mut TargetScanResult,
    stack: &'a mut Vec<PathBuf>,
    next_item_seq: &'a mut u64,
}

fn scan_one_root_iterative(
    target: &TargetDefinition,
    root: &CanonicalPath,
    min_age: Duration,
    now: SystemTime,
    out: &mut TargetScanResult,
    next_item_seq: &mut u64,
) {
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(root.as_str())];
    let mut ctx = ScanWalkCtx {
        target,
        min_age,
        now,
        out,
        stack: &mut stack,
        next_item_seq,
    };
    while let Some(dir) = ctx.stack.pop() {
        let entries = match sweeper_winfs::read_dir_entries(&dir) {
            Ok(v) => v,
            Err(e) => {
                bump_skip_with_context(
                    ctx.out,
                    SkipReason::Unreadable,
                    format!("read_dir_entries {}: {e}", dir.to_string_lossy()),
                );
                continue;
            }
        };
        for ent in entries {
            process_entry(&mut ctx, &ent);
        }
    }
}

fn process_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    match ent.reparse {
        ReparseTagClass::LinkPoint => {
            bump_skip(ctx.out, SkipReason::LinkPoint);
            return;
        }
        ReparseTagClass::CloudPlaceholder => {
            bump_skip(ctx.out, SkipReason::CloudPlaceholder);
            return;
        }
        ReparseTagClass::OtherReparse(_) => {
            bump_skip(ctx.out, SkipReason::OtherReparse);
            return;
        }
        ReparseTagClass::NotReparse => {}
    }

    if ent.is_dir {
        // Re-check via handle reparse inspection for defense-in-depth (no follow).
        match sweeper_winfs::open_for_inspection(&ent.path)
            .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        {
            Ok(ReparseTagClass::LinkPoint) => {
                bump_skip(ctx.out, SkipReason::LinkPoint);
                return;
            }
            Ok(ReparseTagClass::CloudPlaceholder) => {
                bump_skip(ctx.out, SkipReason::CloudPlaceholder);
                return;
            }
            Ok(ReparseTagClass::OtherReparse(_)) => {
                bump_skip(ctx.out, SkipReason::OtherReparse);
                return;
            }
            Ok(ReparseTagClass::NotReparse) => {}
            Err(e) => {
                bump_skip_with_context(
                    ctx.out,
                    SkipReason::Unreadable,
                    format!("inspect_reparse_point {}: {e}", ent.path.to_string_lossy()),
                );
                return;
            }
        }
        ctx.stack.push(ent.path.clone());
        return;
    }

    // Regular file entry.
    let size = ent.size;
    let mtime_secs = filetime_to_unix_secs(ent.last_write_time);
    let mtime_st = UNIX_EPOCH + Duration::from_secs(mtime_secs);
    let future = mtime_st > ctx.now;
    let old_enough = !future
        && ctx
            .now
            .duration_since(mtime_st)
            .map(|d| d >= ctx.min_age)
            .unwrap_or(false);

    let file_identity = sweeper_winfs::open_for_inspection(&ent.path)
        .and_then(|h| sweeper_winfs::get_file_identity(&h))
        .ok();

    *ctx.next_item_seq += 1;
    let item = ScanItem {
        id: format!("{}::{}", ctx.target.id, *ctx.next_item_seq),
        target_id: ctx.target.id.clone(),
        path: ent.path.to_string_lossy().to_string(),
        file_identity,
        size,
        mtime_unix_secs: mtime_secs,
        age_eligible: old_enough,
    };
    ctx.out.total_bytes = ctx.out.total_bytes.saturating_add(size);
    ctx.out.file_count = ctx.out.file_count.saturating_add(1);
    if old_enough {
        ctx.out.eligible_bytes = ctx.out.eligible_bytes.saturating_add(size);
        ctx.out.eligible_count = ctx.out.eligible_count.saturating_add(1);
    }
    ctx.out.items.push(item);
}

fn bump_skip(out: &mut TargetScanResult, reason: SkipReason) {
    bump_skip_with_context(out, reason, String::new());
}

fn bump_skip_with_context(out: &mut TargetScanResult, reason: SkipReason, context: String) {
    out.skipped_count = out.skipped_count.saturating_add(1);
    let entry = out
        .skipped_reasons
        .entry(reason.as_str().to_string())
        .or_insert(0);
    *entry = entry.saturating_add(1);
    if !context.is_empty() {
        out.skipped_diagnostics
            .push(format!("{}: {}", reason.as_str(), context));
    }
}

fn validate_and_collect_roots_for_target(
    target: &TargetDefinition,
    known_folder_root: &CanonicalPath,
    override_root: Option<&CanonicalPath>,
) -> Result<Vec<(String, CanonicalPath)>, String> {
    if target.id == "redirected-temp" {
        if let Some(p) = override_root {
            let root = validate_single_resolved_root(target, p.clone())?;
            return Ok(vec![("redirected-temp".to_string(), root)]);
        }
    }

    let rel = &target.relative_subpath;
    let segments: Vec<&str> = rel.split('\\').collect();
    let mut candidates = vec![PathBuf::from(known_folder_root.as_str())];
    for seg in segments {
        if seg.contains('*') {
            let mut next = Vec::new();
            for cand in candidates {
                if !cand.exists() {
                    continue;
                }
                let entries = match sweeper_winfs::read_dir_entries(&cand) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                for ent in entries {
                    if !ent.is_dir {
                        continue;
                    }
                    let name = ent.name.to_string_lossy();
                    if !segment_pattern_matches(&name, seg) {
                        continue;
                    }
                    // Intermediate wildcard expansion: only non-reparse directories
                    if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
                        continue;
                    }
                    // Defense-in-depth: ensure not a link point via handle.
                    if sweeper_winfs::open_for_inspection(&ent.path)
                        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
                        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    next.push(ent.path);
                }
            }
            candidates = next;
        } else {
            for c in &mut candidates {
                c.push(seg);
            }
        }
    }

    let mut roots = Vec::new();
    for c in candidates {
        if !c.exists() {
            continue;
        }
        match validate_resolved_root_path(&c) {
            Ok(p) => roots.push((target.id.clone(), p)),
            Err(e) => return Err(format!("resolved target root invalid: {e}")),
        }
    }
    roots.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    roots.dedup_by(|a, b| a.1.is_same_path(&b.1));
    Ok(roots)
}

fn validate_single_resolved_root(
    _target: &TargetDefinition,
    path: CanonicalPath,
) -> Result<CanonicalPath, String> {
    validate_resolved_canonical_root(&path).map(|_| path)
}

fn validate_resolved_root_path(path: &Path) -> Result<CanonicalPath, String> {
    let validated =
        CanonicalPath::from_existing_path(&path.to_string_lossy()).map_err(|e| e.to_string())?;
    let canon = validated.into_canonical();
    validate_resolved_canonical_root(&canon)?;
    Ok(canon)
}

fn validate_resolved_canonical_root(root: &CanonicalPath) -> Result<(), String> {
    if root.segment_count() < 3 {
        return Err("resolved target root must have depth >= 3 segments".to_string());
    }
    if policy::is_subtree_protected(root) {
        return Err("resolved target root is subtree-protected".to_string());
    }
    if policy::is_ancestor_of_subtree_protected(root) {
        return Err("resolved target root is ancestor of a subtree-protected path".to_string());
    }
    if policy::is_protected_container_root(root) {
        return Err("resolved target root is a protected container root".to_string());
    }
    if sweeper_winfs::open_for_inspection(Path::new(root.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(false)
    {
        return Err("resolved target root is a link point".to_string());
    }
    // Reject roots that traverse a link point ancestor.
    if path_has_link_point_ancestor(root)? {
        return Err("resolved target root traverses a link point ancestor".to_string());
    }
    Ok(())
}

fn validate_target_roots_policy(
    roots: &[(String, CanonicalPath)],
    all_existing: &[(String, CanonicalPath)],
) -> Result<(), String> {
    for (_, r) in roots {
        // Also ensure not equal/under subtree-protected in broader sense? is_subtree_protected covers exact/descendants.
        if policy::is_subtree_protected(r) || policy::is_ancestor_of_subtree_protected(r) {
            return Err(format!(
                "target root not allowed by protected path policy: {r}"
            ));
        }
        if policy::is_protected_container_root(r) {
            return Err(format!("target root is protected container root: {r}"));
        }
    }
    // Zero unintended overlap between resolved targets (same or ancestor/descendant).
    for (_, a) in roots {
        for (_, b) in all_existing {
            if paths_overlap(a, b) {
                return Err(format!(
                    "overlap between resolved targets detected: {} and {}",
                    a, b
                ));
            }
        }
    }
    for i in 0..roots.len() {
        for j in (i + 1)..roots.len() {
            if paths_overlap(&roots[i].1, &roots[j].1) {
                return Err(format!(
                    "overlap between resolved targets detected: {} and {}",
                    roots[i].1, roots[j].1
                ));
            }
        }
    }
    Ok(())
}

fn paths_overlap(a: &CanonicalPath, b: &CanonicalPath) -> bool {
    a.is_same_path(b) || a.is_self_or_ancestor_of(b) || b.is_self_or_ancestor_of(a)
}

fn segment_pattern_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return name.eq_ignore_ascii_case(pattern);
    }
    // Allowlist lint enforces a single '*' in intermediate segment patterns.
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() != 2 {
        return false;
    }
    let (prefix, suffix) = (parts[0], parts[1]);
    let name_len = name.len();
    let pre_len = prefix.len();
    let suf_len = suffix.len();
    if name_len < pre_len + suf_len {
        return false;
    }
    name[..pre_len].eq_ignore_ascii_case(prefix)
        && name[name_len - suf_len..].eq_ignore_ascii_case(suffix)
}

fn resolve_validated_known_folder_root(kf: KnownFolder) -> Result<CanonicalPath, String> {
    let id = map_known_folder(kf);
    let path = sweeper_winfs::resolve_known_folder(id).map_err(|e| e.to_string())?;
    let canon = CanonicalPath::from_existing_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())?
        .into_canonical();
    // Known folder roots themselves can be redirected; enforce depth/protected checks for key roots.
    match kf {
        KnownFolder::UserProfile | KnownFolder::LocalAppData | KnownFolder::RoamingAppData => {
            if canon.segment_count() < 3 {
                return Err("known folder root depth < 3".to_string());
            }
            if policy::is_subtree_protected(&canon)
                || policy::is_ancestor_of_subtree_protected(&canon)
                || policy::is_protected_container_root(&canon)
            {
                return Err("known folder root violates protected path policy".to_string());
            }
            if path_has_link_point_ancestor(&canon)? {
                return Err("known folder root traverses a link point".to_string());
            }
        }
        KnownFolder::Windows | KnownFolder::ProgramData => {
            // These are container roots in general; allowed as anchors for allowlisted subfolders only.
            // We still require canonical resolution succeeded.
        }
    }
    Ok(canon)
}

fn path_has_link_point_ancestor(path: &CanonicalPath) -> Result<bool, String> {
    // Check each existing prefix path component from root downward (excluding drive root string form edge cases).
    let drive = path.drive_prefix();
    let mut cur = PathBuf::from(format!(r"{}\", drive));
    for seg in path.segments() {
        cur.push(seg);
        match sweeper_winfs::open_for_inspection(&cur)
            .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        {
            Ok(ReparseTagClass::LinkPoint) => return Ok(true),
            Ok(_) => {}
            Err(WinfsError::Win32 { code: 2 | 3, .. }) => break, // not found (shouldn't happen for validated root)
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}

fn evaluate_redirected_temp(local_appdata_root: &CanonicalPath) -> TempGuard {
    let user_temp = match join_and_canonicalize(local_appdata_root, "Temp") {
        Ok(p) => p,
        Err(e) => return TempGuard::Disabled(format!("failed to resolve user-temp: {e}")),
    };
    let temp_env = sweeper_winfs::env_var("TEMP").filter(|v| !v.trim().is_empty());
    evaluate_redirected_temp_candidate(temp_env.as_deref(), &user_temp)
}

/// Evaluate redirected `%TEMP%` guard for an injected candidate path and canonical user-temp root.
///
/// This is a deterministic, testable seam (no global env mutation). `temp_candidate` should be the
/// `%TEMP%` path value to validate, or `None` when unset/empty.
pub fn evaluate_redirected_temp_candidate(
    temp_candidate: Option<&str>,
    user_temp_canonical: &CanonicalPath,
) -> TempGuard {
    let Some(temp_env) = temp_candidate.filter(|v| !v.trim().is_empty()) else {
        return TempGuard::Disabled("%TEMP% not set".to_string());
    };
    if let Ok(root) = crate::paths::parse_drive_root(temp_env.trim()) {
        // Drive root (e.g. C:\) is always depth < 3 and protected; auto-disable deterministically.
        let _ = root;
        return TempGuard::Disabled("depth < 3 (drive root)".to_string());
    }
    let temp_canon = match CanonicalPath::from_existing_path(temp_env) {
        Ok(v) => v.into_canonical(),
        Err(e) => return TempGuard::Disabled(format!("%TEMP% unresolved: {e}")),
    };
    if temp_canon.is_same_path(user_temp_canonical) {
        return TempGuard::Disabled("identical to user-temp".to_string());
    }
    if temp_canon.segment_count() < 3 {
        return TempGuard::Disabled("depth < 3".to_string());
    }
    if policy::is_subtree_protected(&temp_canon)
        || policy::is_ancestor_of_subtree_protected(&temp_canon)
        || policy::is_protected_container_root(&temp_canon)
    {
        return TempGuard::Disabled("resolves to a protected path".to_string());
    }
    match path_has_link_point_ancestor(&temp_canon) {
        Ok(true) => return TempGuard::Disabled("traverses a link point".to_string()),
        Ok(false) => {}
        Err(e) => return TempGuard::Disabled(format!("link-point inspection failed: {e}")),
    }
    // Ensure not a link point itself.
    if sweeper_winfs::open_for_inspection(Path::new(temp_canon.as_str()))
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map(|c| matches!(c, ReparseTagClass::LinkPoint))
        .unwrap_or(false)
    {
        return TempGuard::Disabled("%TEMP% path is a link point".to_string());
    }
    TempGuard::Use(temp_canon)
}

fn join_and_canonicalize(base: &CanonicalPath, child: &str) -> Result<CanonicalPath, String> {
    let p = Path::new(base.as_str()).join(child);
    CanonicalPath::from_existing_path(&p.to_string_lossy())
        .map(|v| v.into_canonical())
        .map_err(|e| e.to_string())
}

fn map_known_folder(kf: KnownFolder) -> KnownFolderId {
    match kf {
        KnownFolder::LocalAppData => KnownFolderId::LocalAppData,
        KnownFolder::RoamingAppData => KnownFolderId::RoamingAppData,
        KnownFolder::UserProfile => KnownFolderId::UserProfile,
        KnownFolder::Windows => KnownFolderId::Windows,
        KnownFolder::ProgramData => KnownFolderId::ProgramData,
    }
}

fn filetime_to_unix_secs(ft: (u32, u32)) -> u64 {
    let v = ((ft.0 as u64) << 32) | (ft.1 as u64);
    // FILETIME is 100-ns intervals since 1601-01-01 UTC.
    const EPOCH_DIFF_100NS: u64 = 116_444_736_000_000_000;
    if v <= EPOCH_DIFF_100NS {
        0
    } else {
        (v - EPOCH_DIFF_100NS) / 10_000_000
    }
}

fn system_time_to_unix_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Render a human-readable report for CLI output.
pub fn format_report(report: &ScanReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Sweeper read-only scan report (min_age_hours={})\n",
        report.min_age_hours
    ));
    out.push_str(&format!(
        "started_unix_secs={}\n\n",
        report.started_unix_secs
    ));
    for t in &report.targets {
        out.push_str(&format!(
            "target_id={} tier={:?} risk={:?}\n",
            t.target_id, t.tier, t.risk
        ));
        out.push_str(&format!("  name={}\n", t.target_name));
        out.push_str(&format!("  scanned={}\n", t.scanned));
        if let Some(r) = &t.status_reason {
            out.push_str(&format!("  status={r}\n"));
        }
        if t.resolved_paths.is_empty() {
            out.push_str("  resolved_paths=[]\n");
        } else {
            out.push_str("  resolved_paths:\n");
            for p in &t.resolved_paths {
                out.push_str(&format!("    - {p}\n"));
            }
        }
        out.push_str(&format!(
            "  total_bytes={} eligible_bytes={} file_count={} eligible_count={} skipped_count={}\n",
            t.total_bytes, t.eligible_bytes, t.file_count, t.eligible_count, t.skipped_count
        ));
        if !t.skipped_reasons.is_empty() {
            out.push_str("  skipped_reasons:\n");
            for (k, v) in &t.skipped_reasons {
                out.push_str(&format!("    - {k}: {v}\n"));
            }
        }
        if !t.skipped_diagnostics.is_empty() {
            out.push_str("  skipped_diagnostics:\n");
            for d in &t.skipped_diagnostics {
                out.push_str(&format!("    - {d}\n"));
            }
        }
        out.push('\n');
    }
    out
}

fn validate_target_definitions(targets: &[TargetDefinition]) -> Result<(), String> {
    let lint_targets: Vec<sweeper_knowledge::LintTarget> = targets
        .iter()
        .map(target_definition_to_lint_target)
        .collect();
    sweeper_knowledge::lint_targets(&lint_targets).map_err(|e| e.to_string())
}

fn target_definition_to_lint_target(t: &TargetDefinition) -> sweeper_knowledge::LintTarget {
    sweeper_knowledge::LintTarget {
        id: t.id.clone(),
        name: t.name.clone(),
        tier: match t.tier {
            Tier::Tier1 => "Tier1".to_string(),
            Tier::Tier2 => "Tier2".to_string(),
            Tier::Tier3 => "Tier3".to_string(),
        },
        risk: match t.risk {
            Risk::Safe => "Safe".to_string(),
            Risk::Caution => "Caution".to_string(),
            Risk::Unknown => "Unknown".to_string(),
            Risk::Protected => "Protected".to_string(),
        },
        known_folder: match t.known_folder {
            KnownFolder::LocalAppData => "LocalAppData".to_string(),
            KnownFolder::RoamingAppData => "RoamingAppData".to_string(),
            KnownFolder::UserProfile => "UserProfile".to_string(),
            KnownFolder::Windows => "Windows".to_string(),
            KnownFolder::ProgramData => "ProgramData".to_string(),
        },
        relative_subpath: t.relative_subpath.clone(),
        scope: "contents".to_string(),
        min_age_hours: t.min_age_hours,
        description: t.description.clone(),
        requires_elevation: t.requires_elevation,
    }
}

/// Verify Gate E read-only invariants against a scan report produced from embedded Tier 1/2 targets.
///
/// Returns a list of violation messages (empty means pass).
pub fn gate_e_violations(report: &ScanReport) -> Vec<String> {
    let mut violations = Vec::new();
    let mut seen_roots: Vec<(String, CanonicalPath)> = Vec::new();
    for t in &report.targets {
        if !t.scanned {
            continue;
        }
        for p in &t.resolved_paths {
            let canon = match CanonicalPath::from_existing_path(p) {
                Ok(v) => v.into_canonical(),
                Err(e) => {
                    violations.push(format!(
                        "target {} resolved path not resolvable: {} ({e})",
                        t.target_id, p
                    ));
                    continue;
                }
            };
            if policy::is_subtree_protected(&canon)
                || policy::is_ancestor_of_subtree_protected(&canon)
            {
                violations.push(format!(
                    "target {} overlaps subtree-protected path: {}",
                    t.target_id, canon
                ));
            }
            if policy::is_protected_container_root(&canon) {
                violations.push(format!(
                    "target {} targets protected container root: {}",
                    t.target_id, canon
                ));
            }
            match sweeper_winfs::open_for_inspection(Path::new(canon.as_str()))
                .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
            {
                Ok(ReparseTagClass::LinkPoint) => violations.push(format!(
                    "target {} resolved root is link point: {}",
                    t.target_id, canon
                )),
                Ok(_) => {}
                Err(e) => violations.push(format!(
                    "target {} failed reparse inspection for {}: {e}",
                    t.target_id, canon
                )),
            }
            for (other_id, other) in &seen_roots {
                if paths_overlap(&canon, other) {
                    violations.push(format!(
                        "overlap between resolved targets: {} ({}) and {} ({})",
                        t.target_id, canon, other_id, other
                    ));
                }
            }
            seen_roots.push((t.target_id.clone(), canon));
        }
        // Zero link points followed is enforced by scanner behavior; report includes skip accounting.
        // We assert no impossible state here; scanner never traverses link points.
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use sweeper_knowledge::{KnownFolder, Risk, Scope, Tier};

    fn sample_target(id: &str, rel: &str) -> TargetDefinition {
        TargetDefinition {
            id: id.to_string(),
            name: id.to_string(),
            tier: Tier::Tier1,
            risk: Risk::Safe,
            known_folder: KnownFolder::LocalAppData,
            relative_subpath: rel.to_string(),
            scope: Scope::Contents,
            min_age_hours: 24,
            description: "test".to_string(),
            requires_elevation: false,
        }
    }

    #[test]
    fn rejects_caller_supplied_target_shapes_via_lint() {
        let bad = vec![sample_target("bad", "Foo\\**\\Bar")];
        let err = scan_targets(bad, 24).unwrap_err().to_string();
        assert!(err.contains("**") || err.contains("glob"), "{err}");
    }

    #[test]
    fn redirected_temp_guard_depth_lt_3_drive_root_disabled() {
        let local = sweeper_winfs::resolve_known_folder(KnownFolderId::LocalAppData)
            .expect("resolve local appdata");
        let user_temp = CanonicalPath::from_existing_path(&local.join("Temp").to_string_lossy())
            .expect("user temp")
            .into_canonical();
        let guard = evaluate_redirected_temp_candidate(Some(r"C:\"), &user_temp);
        match guard {
            TempGuard::Disabled(reason) => assert!(reason.contains("depth < 3"), "{reason}"),
            TempGuard::Use(p) => panic!("expected disabled, got {p}"),
        }
    }

    #[test]
    fn redirected_temp_guard_identical_to_user_temp_disabled() {
        let local = sweeper_winfs::resolve_known_folder(KnownFolderId::LocalAppData)
            .expect("resolve local appdata");
        let user_temp = CanonicalPath::from_existing_path(&local.join("Temp").to_string_lossy())
            .expect("user temp")
            .into_canonical();
        let guard = evaluate_redirected_temp_candidate(Some(user_temp.as_str()), &user_temp);
        match guard {
            TempGuard::Disabled(reason) => assert!(reason.contains("identical"), "{reason}"),
            TempGuard::Use(p) => panic!("expected disabled, got {p}"),
        }
    }

    #[test]
    fn redirected_temp_guard_profile_root_disabled_as_protected_container() {
        let local = sweeper_winfs::resolve_known_folder(KnownFolderId::LocalAppData)
            .expect("resolve local appdata");
        let user_temp = CanonicalPath::from_existing_path(&local.join("Temp").to_string_lossy())
            .expect("user temp")
            .into_canonical();
        let profile = sweeper_winfs::resolve_known_folder(KnownFolderId::UserProfile)
            .expect("resolve profile");
        let guard =
            evaluate_redirected_temp_candidate(Some(&profile.to_string_lossy()), &user_temp);
        match guard {
            TempGuard::Disabled(reason) => {
                assert!(
                    reason.contains("protected") || reason.contains("depth < 3"),
                    "{reason}"
                )
            }
            TempGuard::Use(p) => panic!("expected disabled, got {p}"),
        }
    }
}
