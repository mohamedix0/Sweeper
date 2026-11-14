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
    scan_targets_with_redirected_temp_and_elevation_overrides(
        targets,
        min_age_hours,
        redirected_temp_candidate,
        &BTreeMap::new(),
    )
}

/// Scan a provided set of targets with optional redirected-temp and elevation-target root overrides.
///
/// Elevation overrides are only applied to `requires_elevation` targets and are validated with the
/// same safety checks as normal resolved roots (non-bypassing test/harness seam).
pub fn scan_targets_with_redirected_temp_and_elevation_overrides(
    targets: Vec<TargetDefinition>,
    min_age_hours: u32,
    redirected_temp_candidate: Option<&str>,
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> Result<ScanReport, ScanError> {
    validate_target_definitions(&targets).map_err(ScanError::Targets)?;
    let now = SystemTime::now();
    let started_unix_secs = system_time_to_unix_secs(now);
    let min_age = Duration::from_secs(u64::from(min_age_hours) * 3600);

    // Resolve known folder roots used by these targets.
    let needed_kfs: HashSet<KnownFolder> = targets
        .iter()
        .filter(|t| !(t.requires_elevation && elevation_target_root_overrides.contains_key(&t.id)))
        .map(|t| t.known_folder)
        .collect();
    let mut kf_roots: BTreeMap<KnownFolder, Result<CanonicalPath, String>> = BTreeMap::new();
    for kf in needed_kfs {
        kf_roots.insert(kf, resolve_validated_known_folder_root(kf));
    }

    let mut results = Vec::with_capacity(targets.len());
    let mut all_resolved_roots: Vec<(String, CanonicalPath)> = Vec::new();

    for target in targets {
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

        // Test/harness seam: override resolved root for audited elevation targets only.
        if target.requires_elevation {
            if let Some(override_path) = elevation_target_root_overrides.get(&target.id) {
                match CanonicalPath::from_existing_path(override_path) {
                    Ok(v) => {
                        let canon = v.into_canonical();
                        match validate_resolved_canonical_root(&canon) {
                            Ok(()) => {
                                let roots = vec![(target.id.clone(), canon)];
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
                                out.resolved_paths =
                                    roots.iter().map(|(_, p)| p.to_string()).collect();
                                scan_resolved_roots(&target, &roots, min_age, now, &mut out);
                                results.push(out);
                                continue;
                            }
                            Err(e) => {
                                out.status_reason =
                                    Some(format!("elevation target root override invalid: {e}"));
                                results.push(out);
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        out.status_reason =
                            Some(format!("elevation target root override unresolved: {e}"));
                        results.push(out);
                        continue;
                    }
                }
            }
        }

        let kf_res = kf_roots
            .get(&target.known_folder)
            .cloned()
            .unwrap_or_else(|| Err("known folder not resolved".to_string()));
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
    process_directory_entry(ctx, ent);
}

fn process_directory_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    if let Some(reason) = skip_reason_for_reparse(ent.reparse) {
        bump_skip(ctx.out, reason);
        return;
    }
    if ent.is_dir {
        handle_directory_entry(ctx, ent);
        return;
    }
    record_file_entry(ctx, ent);
}

fn skip_reason_for_reparse(reparse: ReparseTagClass) -> Option<SkipReason> {
    match reparse {
        ReparseTagClass::LinkPoint => Some(SkipReason::LinkPoint),
        ReparseTagClass::CloudPlaceholder => Some(SkipReason::CloudPlaceholder),
        ReparseTagClass::OtherReparse(_) => Some(SkipReason::OtherReparse),
        ReparseTagClass::NotReparse => None,
    }
}

fn handle_directory_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    match inspect_directory_reparse(ent) {
        Ok(None) => ctx.stack.push(ent.path.clone()),
        Ok(Some(reason)) => bump_skip(ctx.out, reason),
        Err(msg) => bump_skip_with_context(ctx.out, SkipReason::Unreadable, msg),
    }
}

fn inspect_directory_reparse(ent: &DirEntry) -> Result<Option<SkipReason>, String> {
    let class = sweeper_winfs::open_for_inspection(&ent.path)
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
        .map_err(|e| format!("inspect_reparse_point {}: {e}", ent.path.to_string_lossy()))?;
    Ok(skip_reason_for_reparse(class))
}

fn record_file_entry(ctx: &mut ScanWalkCtx<'_>, ent: &DirEntry) {
    let mtime_secs = filetime_to_unix_secs(ent.last_write_time);
    let old_enough = is_file_age_eligible(mtime_secs, ctx.now, ctx.min_age);
    *ctx.next_item_seq += 1;
    let item = build_scan_item(ctx, ent, mtime_secs, old_enough);
    accumulate_file_item(ctx.out, &item);
    ctx.out.items.push(item);
}

fn is_file_age_eligible(mtime_secs: u64, now: SystemTime, min_age: Duration) -> bool {
    let mtime_st = UNIX_EPOCH + Duration::from_secs(mtime_secs);
    if mtime_st > now {
        return false;
    }
    now.duration_since(mtime_st)
        .map(|d| d >= min_age)
        .unwrap_or(false)
}

fn build_scan_item(
    ctx: &ScanWalkCtx<'_>,
    ent: &DirEntry,
    mtime_secs: u64,
    age_eligible: bool,
) -> ScanItem {
    ScanItem {
        id: format!("{}::{}", ctx.target.id, *ctx.next_item_seq),
        target_id: ctx.target.id.clone(),
        path: ent.path.to_string_lossy().to_string(),
        file_identity: file_identity_for_path(&ent.path),
        size: ent.size,
        mtime_unix_secs: mtime_secs,
        age_eligible,
    }
}

fn file_identity_for_path(path: &Path) -> Option<FileIdentity> {
    sweeper_winfs::open_for_inspection(path)
        .and_then(|h| sweeper_winfs::get_file_identity(&h))
        .ok()
}

fn accumulate_file_item(out: &mut TargetScanResult, item: &ScanItem) {
    out.total_bytes = out.total_bytes.saturating_add(item.size);
    out.file_count = out.file_count.saturating_add(1);
    if item.age_eligible {
        out.eligible_bytes = out.eligible_bytes.saturating_add(item.size);
        out.eligible_count = out.eligible_count.saturating_add(1);
    }
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

/// Options for settings-driven scans (custom paths and Tier 3 conditionals).
#[derive(Debug, Clone, Default)]
pub struct ScanSettingsOptions {
    /// Override current time (tests). Uses `SystemTime::now()` when `None`.
    pub now: Option<SystemTime>,
    /// Optional injected redirected `%TEMP%` candidate path for deterministic tests.
    pub redirected_temp_candidate: Option<String>,
    /// Optional injected recycle-bin scan roots (directories whose contents are scanned as Tier 3 recycle-bin).
    /// When empty and Tier 3 recycle-bin scan is requested in production, accessible `$Recycle.Bin\<SID>` dirs are discovered.
    pub recycle_bin_roots: Vec<String>,
    /// Optional injected project search roots for Tier 3 stale `target/` and `node_modules` discovery.
    /// When empty in production opt-in scans, a conservative default (user profile root) is used if available.
    pub tier3_project_search_roots: Vec<String>,
    /// Optional test/harness-only root overrides for audited elevation targets.
    ///
    /// Keys are target ids (e.g. `windows-temp`). When present for a `requires_elevation`
    /// target, the override path is canonicalized and validated with the same safety checks as
    /// any resolved target root before scanning; this enables isolated sentinel-sandbox tests
    /// without touching real system paths.
    pub elevation_target_root_overrides: BTreeMap<String, String>,
}

/// Scan Tier 1/2 allowlisted targets plus validated Custom Paths and opt-in Tier 3 conditional targets.
///
/// - Tier toggles are respected (`enable_tier1/2/3`).
/// - Custom Paths are re-validated at scan time; missing/unsafe paths emit zero items and diagnostics (never fallback to parent).
/// - Tier 3 targets (`recycle-bin`, `stale-rust-target`, `stale-node-modules`) run only when `enable_tier3` is true.
pub fn scan_with_settings(
    settings: &crate::settings::SweeperSettings,
) -> Result<ScanReport, ScanError> {
    scan_with_settings_options(settings, ScanSettingsOptions::default())
}

/// Scan with injectable options for deterministic/testable Tier 3 and custom-path behavior.
pub fn scan_with_settings_options(
    settings: &crate::settings::SweeperSettings,
    opts: ScanSettingsOptions,
) -> Result<ScanReport, ScanError> {
    scan_with_settings_options_and_elevation(settings, opts, crate::ScanElevationContext::default())
}

/// Scan using the current process elevation state (via `sweeper_winfs::is_process_elevated()`).
pub fn scan_with_settings_current_elevation(
    settings: &crate::settings::SweeperSettings,
) -> Result<ScanReport, ScanError> {
    scan_with_settings_options_current_elevation(settings, ScanSettingsOptions::default())
}

/// Scan with options using the current process elevation state.
pub fn scan_with_settings_options_current_elevation(
    settings: &crate::settings::SweeperSettings,
    opts: ScanSettingsOptions,
) -> Result<ScanReport, ScanError> {
    let elevated = sweeper_winfs::is_process_elevated().unwrap_or(false);
    scan_with_settings_options_and_elevation(
        settings,
        opts,
        crate::ScanElevationContext {
            process_elevated: elevated,
        },
    )
}

/// Scan with explicit elevation context (deterministic for tests).
pub fn scan_with_settings_options_and_elevation(
    settings: &crate::settings::SweeperSettings,
    opts: ScanSettingsOptions,
    elevation: crate::ScanElevationContext,
) -> Result<ScanReport, ScanError> {
    let now = opts.now.unwrap_or_else(SystemTime::now);
    let started_unix_secs = system_time_to_unix_secs(now);
    let min_age = Duration::from_secs(u64::from(settings.min_age_hours) * 3600);

    let elev_ok =
        elevation.process_elevated && settings.enable_tier3 && settings.enable_elevation_targets;
    let mut targets: Vec<TargetDefinition> = sweeper_knowledge::embedded_targets()
        .into_iter()
        .filter(|t| {
            if t.requires_elevation {
                elev_ok
            } else {
                match t.tier {
                    Tier::Tier1 => settings.enable_tier1,
                    Tier::Tier2 => settings.enable_tier2,
                    Tier::Tier3 => settings.enable_tier3,
                }
            }
        })
        .collect();

    // If Tier 1/2 are disabled, we still may scan custom paths / Tier 3 conditionals.
    // For allowlisted targets, reuse existing pipeline (it validates/lints and resolves roots).
    let mut report = if targets.is_empty() {
        ScanReport {
            started_unix_secs,
            min_age_hours: settings.min_age_hours,
            targets: Vec::new(),
        }
    } else {
        scan_targets_with_redirected_temp_and_elevation_overrides(
            std::mem::take(&mut targets),
            settings.min_age_hours,
            opts.redirected_temp_candidate.as_deref(),
            &opts.elevation_target_root_overrides,
        )?
    };
    report.started_unix_secs = started_unix_secs;
    report.min_age_hours = settings.min_age_hours;

    append_custom_path_targets(&mut report, settings, min_age, now);
    append_tier3_targets(&mut report, settings, &opts, min_age, now);

    Ok(report)
}

fn scan_custom_path_target(
    idx: usize,
    raw_path: &str,
    min_age: Duration,
    now: SystemTime,
) -> TargetScanResult {
    let target_id = format!("custom-path-{}", idx + 1);
    let mut out = empty_target_result(
        target_id,
        "Custom Path".to_string(),
        Tier::Tier2, // display grouping tier; custom paths are Caution/contents-only
        Risk::Caution,
    );

    let verdict = policy::validate_custom_path(raw_path);
    if !verdict.allowed {
        out.scanned = false;
        out.status_reason = Some(format!(
            "custom path invalid/missing (no fallback): {}",
            verdict.reason
        ));
        out.skipped_diagnostics.push(format!(
            "custom path rejected: {} ({})",
            raw_path, verdict.reason
        ));
        return out;
    }

    let canon = match CanonicalPath::from_existing_path(raw_path) {
        Ok(v) => v.into_canonical(),
        Err(e) => {
            out.scanned = false;
            out.status_reason = Some(format!(
                "custom path missing/unresolvable (no fallback): {e}"
            ));
            out.skipped_diagnostics
                .push(format!("custom path unresolved: {raw_path} ({e})"));
            return out;
        }
    };

    if let Err(e) = validate_resolved_canonical_root(&canon) {
        out.scanned = false;
        out.status_reason = Some(format!("custom path not allowed at scan time: {e}"));
        out.skipped_diagnostics
            .push(format!("custom path rejected at scan time: {e}"));
        return out;
    }

    let td = TargetDefinition {
        id: out.target_id.clone(),
        name: out.target_name.clone(),
        tier: Tier::Tier2,
        risk: Risk::Caution,
        known_folder: KnownFolder::LocalAppData, // unused for custom resolved scan
        relative_subpath: String::new(),
        scope: sweeper_knowledge::Scope::Contents,
        min_age_hours: (min_age.as_secs() / 3600) as u32,
        description: "User-validated custom path (contents only).".to_string(),
        requires_elevation: false,
    };
    out.resolved_paths = vec![canon.to_string()];
    scan_resolved_roots(
        &td,
        &[(out.target_id.clone(), canon)],
        min_age,
        now,
        &mut out,
    );
    out
}

fn empty_target_result(
    target_id: String,
    target_name: String,
    tier: Tier,
    risk: Risk,
) -> TargetScanResult {
    TargetScanResult {
        target_id,
        target_name,
        tier,
        risk,
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
    }
}

fn append_custom_path_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    min_age: Duration,
    now: SystemTime,
) {
    for (idx, raw) in settings.custom_paths.iter().enumerate() {
        report
            .targets
            .push(scan_custom_path_target(idx, raw, min_age, now));
    }
}

fn append_tier3_targets(
    report: &mut ScanReport,
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
) {
    if !settings.enable_tier3 {
        return;
    }
    report
        .targets
        .extend(scan_tier3_conditional_targets(settings, opts, min_age, now));
}

fn scan_tier3_conditional_targets(
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
) -> Vec<TargetScanResult> {
    let mut out = Vec::new();
    out.push(scan_tier3_recycle_bin(opts, min_age, now));
    out.extend(scan_tier3_stale_project_dirs(settings, opts, min_age, now));
    out
}

fn scan_tier3_recycle_bin(
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
) -> TargetScanResult {
    let mut result = empty_target_result(
        "recycle-bin".to_string(),
        "Recycle Bin (current user)".to_string(),
        Tier::Tier3,
        Risk::Caution,
    );

    let roots = if !opts.recycle_bin_roots.is_empty() {
        opts.recycle_bin_roots.clone()
    } else {
        discover_recycle_bin_roots()
    };

    let mut valid_roots: Vec<(String, CanonicalPath)> = Vec::new();
    for r in roots {
        if r.trim().is_empty() {
            continue;
        }
        match CanonicalPath::from_existing_path(&r) {
            Ok(v) => {
                let canon = v.into_canonical();
                match validate_recycle_bin_root(&canon) {
                    Ok(()) => valid_roots.push((result.target_id.clone(), canon)),
                    Err(e) => result
                        .skipped_diagnostics
                        .push(format!("recycle-bin root skipped: {} ({e})", canon)),
                }
            }
            Err(e) => result
                .skipped_diagnostics
                .push(format!("recycle-bin root unresolved: {r} ({e})")),
        }
    }

    if valid_roots.is_empty() {
        result.scanned = false;
        result.status_reason = Some("no accessible user recycle-bin root found".to_string());
        return result;
    }

    valid_roots.sort_by(|a, b| a.1.as_str().cmp(b.1.as_str()));
    valid_roots.dedup_by(|a, b| a.1.is_same_path(&b.1));
    result.resolved_paths = valid_roots.iter().map(|(_, p)| p.to_string()).collect();

    let td = TargetDefinition {
        id: "recycle-bin".to_string(),
        name: "Recycle Bin (current user)".to_string(),
        tier: Tier::Tier3,
        risk: Risk::Caution,
        known_folder: KnownFolder::Windows, // unused
        relative_subpath: String::new(),
        scope: sweeper_knowledge::Scope::Contents,
        min_age_hours: (min_age.as_secs() / 3600) as u32,
        description: "Permanently empties user-accessible Recycle Bin contents (opt-in)."
            .to_string(),
        requires_elevation: false,
    };
    scan_resolved_roots(&td, &valid_roots, min_age, now, &mut result);
    result
}

fn validate_recycle_bin_root(canon: &CanonicalPath) -> Result<(), String> {
    let verdict = policy::validate_tier3_target_root(canon.as_str());
    if !verdict.allowed {
        return Err(verdict.reason);
    }
    if path_has_link_point_ancestor(canon)? {
        return Err("traverses link point".to_string());
    }
    Ok(())
}

fn discover_recycle_bin_roots() -> Vec<String> {
    let mut out = Vec::new();
    let system_drive = sweeper_winfs::env_var("SystemDrive").unwrap_or_else(|| "C:".to_string());
    let base = PathBuf::from(format!(
        r"{}\$Recycle.Bin",
        system_drive.trim_end_matches('\\')
    ));
    if !base.exists() {
        return out;
    }
    let entries = match sweeper_winfs::read_dir_entries(&base) {
        Ok(v) => v,
        Err(_) => return out,
    };
    for ent in entries {
        if !ent.is_dir {
            continue;
        }
        if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
            continue;
        }
        let p = ent.path.to_string_lossy().to_string();
        if let Ok(v) = CanonicalPath::from_existing_path(&p) {
            let canon = v.into_canonical();
            // Prefer SID-like subfolders (S-1-*) when discoverable; still only include handle-accessible safe ones.
            let name = ent.name.to_string_lossy();
            let sid_like = name.starts_with("S-1-");
            if !sid_like {
                // Allow non-SID named dirs only if accessible and safe (best-effort); keep conservative.
            }
            if validate_recycle_bin_root(&canon).is_ok() {
                out.push(canon.to_string());
            }
        }
    }
    out
}

fn scan_tier3_stale_project_dirs(
    settings: &crate::settings::SweeperSettings,
    opts: &ScanSettingsOptions,
    min_age: Duration,
    now: SystemTime,
) -> Vec<TargetScanResult> {
    let mut out = Vec::new();
    let search_roots = if !opts.tier3_project_search_roots.is_empty() {
        opts.tier3_project_search_roots.clone()
    } else {
        default_tier3_project_search_roots()
    };

    let stale_days = settings.stale_project_days;
    let stale_threshold = Duration::from_secs(u64::from(stale_days) * 24 * 3600);

    let mut rust_dirs: Vec<(String, u64, u64, u64, u64, u64)> = Vec::new(); // path,total,eligible,file,eligible_count,mtime
    let mut node_dirs: Vec<(String, u64, u64, u64, u64, u64)> = Vec::new();

    for root in search_roots {
        discover_stale_project_dirs(&root, stale_threshold, now, &mut rust_dirs, &mut node_dirs);
    }

    out.push(build_stale_project_target_result(
        "stale-rust-target",
        "Stale Rust target/",
        rust_dirs,
        min_age,
    ));
    out.push(build_stale_project_target_result(
        "stale-node-modules",
        "Stale node_modules/",
        node_dirs,
        min_age,
    ));
    out
}

fn default_tier3_project_search_roots() -> Vec<String> {
    let mut roots = Vec::new();
    if let Ok(p) = sweeper_winfs::resolve_known_folder(KnownFolderId::UserProfile) {
        if let Ok(v) = CanonicalPath::from_existing_path(&p.to_string_lossy()) {
            let canon = v.into_canonical();
            if !policy::is_subtree_protected(&canon)
                && !policy::is_ancestor_of_subtree_protected(&canon)
                && !policy::is_protected_container_root(&canon)
            {
                roots.push(canon.to_string());
            }
        }
    }
    roots
}

fn discover_stale_project_dirs(
    search_root: &str,
    stale_threshold: Duration,
    now: SystemTime,
    rust_dirs: &mut Vec<(String, u64, u64, u64, u64, u64)>,
    node_dirs: &mut Vec<(String, u64, u64, u64, u64, u64)>,
) {
    let root = match CanonicalPath::from_existing_path(search_root) {
        Ok(v) => v.into_canonical(),
        Err(_) => return,
    };
    if policy::is_subtree_protected(&root)
        || policy::is_ancestor_of_subtree_protected(&root)
        || policy::is_protected_container_root(&root)
    {
        return;
    }
    // Never scan Windows / Program Files etc. (subtree-protected/container roots already blocked)
    let mut stack: Vec<PathBuf> = vec![PathBuf::from(root.as_str())];
    let mut seen: HashSet<String> = HashSet::new();

    while let Some(dir) = stack.pop() {
        let dir_key = dir.to_string_lossy().to_ascii_uppercase();
        if !seen.insert(dir_key) {
            continue;
        }
        if is_disallowed_tier3_scan_path(&dir) {
            continue;
        }
        let entries = match sweeper_winfs::read_dir_entries(&dir) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Check child dirs for stale project artifacts.
        for ent in &entries {
            if !ent.is_dir {
                continue;
            }
            if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
                continue;
            }
            let name = ent.name.to_string_lossy();
            if name.eq_ignore_ascii_case("target") {
                if let Some(info) = stale_rust_target_info(&ent.path, stale_threshold, now) {
                    rust_dirs.push(info);
                }
            } else if name.eq_ignore_ascii_case("node_modules") {
                if let Some(info) = stale_node_modules_info(&ent.path, stale_threshold, now) {
                    node_dirs.push(info);
                }
            }
        }
        // Traverse deeper for discovery, but skip link points/protected/disallowed dirs and known heavy dirs.
        for ent in entries {
            if !ent.is_dir {
                continue;
            }
            if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
                continue;
            }
            let name = ent.name.to_string_lossy();
            if name.eq_ignore_ascii_case("$Recycle.Bin")
                || name.eq_ignore_ascii_case("System Volume Information")
                || name.eq_ignore_ascii_case("Windows")
                || name.eq_ignore_ascii_case("Program Files")
                || name.eq_ignore_ascii_case("Program Files (x86)")
            {
                continue;
            }
            // Avoid descending into discovered stale dirs themselves.
            if name.eq_ignore_ascii_case("target") && ent.path.join("..\\Cargo.toml").exists() {
                continue;
            }
            if name.eq_ignore_ascii_case("node_modules") {
                continue;
            }
            if !is_disallowed_tier3_scan_path(&ent.path) {
                stack.push(ent.path);
            }
        }
    }

    rust_dirs.sort_by(|a, b| a.0.cmp(&b.0));
    rust_dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
    node_dirs.sort_by(|a, b| a.0.cmp(&b.0));
    node_dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
}

fn is_disallowed_tier3_scan_path(path: &Path) -> bool {
    let Ok(canon) = CanonicalPath::parse_lexical(&path.to_string_lossy()) else {
        return true;
    };
    if policy::is_subtree_protected(&canon)
        || policy::is_ancestor_of_subtree_protected(&canon)
        || policy::is_protected_container_root(&canon)
    {
        return true;
    }
    // Extra hard blocks by name/path anchors.
    let s = canon.as_str().to_ascii_uppercase();
    s.starts_with(r"C:\WINDOWS")
        || s.starts_with(r"C:\PROGRAM FILES")
        || s.starts_with(r"C:\PROGRAM FILES (X86)")
        || s.contains(r"\SYSTEM VOLUME INFORMATION")
}

fn stale_rust_target_info(
    target_dir: &Path,
    stale_threshold: Duration,
    now: SystemTime,
) -> Option<(String, u64, u64, u64, u64, u64)> {
    let parent = target_dir.parent()?;
    let cargo_toml = parent.join("Cargo.toml");
    if !cargo_toml.is_file() {
        return None;
    }
    if !is_path_stale_enough(&cargo_toml, stale_threshold, now) {
        return None;
    }
    stale_dir_item_info(target_dir)
}

fn stale_node_modules_info(
    node_modules_dir: &Path,
    stale_threshold: Duration,
    now: SystemTime,
) -> Option<(String, u64, u64, u64, u64, u64)> {
    let parent = node_modules_dir.parent()?;
    let lockfiles = [
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
    ];
    let mut found = false;
    for lf in lockfiles {
        let p = parent.join(lf);
        if p.is_file() {
            found = true;
            if !is_path_stale_enough(&p, stale_threshold, now) {
                return None;
            }
        }
    }
    if !found {
        return None;
    }
    stale_dir_item_info(node_modules_dir)
}

fn is_path_stale_enough(path: &Path, stale_threshold: Duration, now: SystemTime) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let mtime = match meta.modified() {
        Ok(t) => t,
        Err(_) => return false,
    };
    if mtime > now {
        return false;
    }
    now.duration_since(mtime)
        .map(|age| age >= stale_threshold)
        .unwrap_or(false)
}

fn stale_dir_item_info(dir: &Path) -> Option<(String, u64, u64, u64, u64, u64)> {
    let canon = CanonicalPath::from_existing_path(&dir.to_string_lossy())
        .ok()?
        .into_canonical();
    // Safety: must be eligible Tier 3 root and not protected/link point.
    if !policy::validate_tier3_target_root(canon.as_str()).allowed {
        return None;
    }
    let (total, eligible, count, eligible_count, mtime) = summarize_directory_for_scan(dir).ok()?;
    Some((
        canon.to_string(),
        total,
        eligible,
        count,
        eligible_count,
        mtime,
    ))
}

fn summarize_directory_for_scan(dir: &Path) -> Result<(u64, u64, u64, u64, u64), String> {
    let mut total: u64 = 0;
    let mut eligible: u64 = 0;
    let mut count: u64 = 0;
    let mut eligible_count: u64 = 0;
    let mut max_mtime: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = sweeper_winfs::read_dir_entries(&d).map_err(|e| e.to_string())?;
        for ent in entries {
            match ent.reparse {
                ReparseTagClass::NotReparse => {}
                _ => continue, // do not traverse/count skipped reparse entries in aggregate
            }
            if ent.is_dir {
                stack.push(ent.path);
            } else {
                let mt = filetime_to_unix_secs(ent.last_write_time);
                max_mtime = max_mtime.max(mt);
                total = total.saturating_add(ent.size);
                eligible = eligible.saturating_add(ent.size); // staleness decided by project marker; treat contents as eligible
                count = count.saturating_add(1);
                eligible_count = eligible_count.saturating_add(1);
            }
        }
    }
    Ok((total, eligible, count, eligible_count, max_mtime))
}

fn build_stale_project_target_result(
    target_id: &str,
    target_name: &str,
    dirs: Vec<(String, u64, u64, u64, u64, u64)>,
    _min_age: Duration,
) -> TargetScanResult {
    let mut result = empty_target_result(
        target_id.to_string(),
        target_name.to_string(),
        Tier::Tier3,
        Risk::Caution,
    );
    if dirs.is_empty() {
        result.scanned = false;
        result.status_reason = Some("no stale directories found".to_string());
        return result;
    }
    result.scanned = true;
    for (i, (path, total, eligible, count, eligible_count, mtime)) in dirs.into_iter().enumerate() {
        result.resolved_paths.push(path.clone());
        result.total_bytes = result.total_bytes.saturating_add(total);
        result.eligible_bytes = result.eligible_bytes.saturating_add(eligible);
        result.file_count = result.file_count.saturating_add(count);
        result.eligible_count = result.eligible_count.saturating_add(eligible_count);
        result.items.push(ScanItem {
            id: format!("{}::{}", target_id, i + 1),
            target_id: target_id.to_string(),
            path,
            file_identity: None,
            size: total,
            mtime_unix_secs: mtime,
            age_eligible: true,
        });
    }
    result
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
