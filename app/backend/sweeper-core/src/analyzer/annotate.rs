use super::bounded::BoundedTopN;
use super::walker::{DirRecord, FileInfo};
use super::*;
use crate::paths::CanonicalPath;
use crate::policy;
use crate::wildcard::segment_pattern_matches;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use sweeper_knowledge::{embedded_targets, KnowledgeEntry, Risk, TargetDefinition, Tier};
use sweeper_winfs::{self, ReparseTagClass};

#[derive(Debug, Clone)]
pub(super) struct AnnotationContext {
    user_profile_root: Option<CanonicalPath>,
    targets: Vec<ResolvedTarget>,
    known: Vec<KnowledgeEntry>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedTarget {
    def: TargetDefinition,
    root: CanonicalPath,
}

#[derive(Debug, Clone)]
pub(super) struct EligibilitySettings {
    pub(super) enabled_tiers: HashSet<Tier>,
    pub(super) allow_unknown: bool,
}

impl Default for EligibilitySettings {
    fn default() -> Self {
        let mut enabled_tiers = HashSet::new();
        enabled_tiers.insert(Tier::Tier1);
        enabled_tiers.insert(Tier::Tier2);
        // Tier 3 disabled by default in v1.
        Self {
            enabled_tiers,
            allow_unknown: false,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Annotation {
    risk: Risk,
    description: String,
    tier: Option<Tier>,
    target_id: Option<String>,
    is_link_point: bool,
    is_cloud_placeholder: bool,
    display_only: bool,
    selectable: bool,
    selection_reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TreeBuildLimits {
    pub(super) max_children_per_dir: usize,
    pub(super) max_tree_depth: usize,
}

fn compute_directory_totals(
    records: &HashMap<String, DirRecord>,
    root_key: &str,
) -> HashMap<String, (u64, u64)> {
    let mut totals: HashMap<String, (u64, u64)> = HashMap::new();
    let mut stack: Vec<(String, bool)> = vec![(root_key.to_string(), false)];
    let mut visited_expand: HashSet<String> = HashSet::new();

    while let Some((key, expanded)) = stack.pop() {
        if expanded {
            let (size, count) = directory_total_for(&key, records, &totals);
            totals.insert(key, (size, count));
            continue;
        }
        if totals.contains_key(&key) {
            continue;
        }
        if !visited_expand.insert(key.clone()) {
            totals.entry(key).or_insert((0, 0));
            continue;
        }
        stack.push((key.clone(), true));
        push_uncomputed_children(&key, records, &totals, &mut stack);
    }
    totals
}

fn directory_total_for(
    key: &str,
    records: &HashMap<String, DirRecord>,
    totals: &HashMap<String, (u64, u64)>,
) -> (u64, u64) {
    let mut size: u64 = 0;
    let mut count: u64 = 0;
    let Some(rec) = records.get(key) else {
        return (0, 0);
    };
    size = size.saturating_add(rec.own_bytes);
    count = count.saturating_add(rec.own_file_count);
    let mut seen_child = HashSet::new();
    for child_path in &rec.children_dirs {
        let ck = canonical_key(child_path);
        if !seen_child.insert(ck.clone()) {
            continue;
        }
        let (cs, cc) = totals.get(&ck).copied().unwrap_or((0, 0));
        size = size.saturating_add(cs);
        count = count.saturating_add(1).saturating_add(cc);
    }
    (size, count)
}

fn push_uncomputed_children(
    key: &str,
    records: &HashMap<String, DirRecord>,
    totals: &HashMap<String, (u64, u64)>,
    stack: &mut Vec<(String, bool)>,
) {
    let Some(rec) = records.get(key) else {
        return;
    };
    let mut seen_child = HashSet::new();
    for child_path in &rec.children_dirs {
        let ck = canonical_key(child_path);
        if !seen_child.insert(ck.clone()) {
            continue;
        }
        if records.contains_key(&ck) && !totals.contains_key(&ck) {
            stack.push((ck, false));
        }
    }
}

fn build_file_node(
    file: &FileInfo,
    ctx: &AnnotationContext,
    settings: &EligibilitySettings,
) -> AnalyzerNode {
    let file_ann = annotate_path(&file.path, false, Some(file), ctx, settings, None);
    AnalyzerNode {
        id: stable_node_id(&file.path),
        path: file.path.clone(),
        name: file.name.clone(),
        is_dir: false,
        size_bytes: file.size,
        item_count: 1,
        risk: file_ann.risk,
        description: file_ann.description,
        tier: file_ann.tier,
        target_id: file_ann.target_id,
        is_link_point: file_ann.is_link_point,
        is_cloud_placeholder: file_ann.is_cloud_placeholder,
        display_only: file_ann.display_only,
        selectable: file_ann.selectable,
        selection_reason: file_ann.selection_reason,
        children: Vec::new(),
    }
}

/// Prune key computed once per candidate child. Ordering is size descending,
/// then precomputed lowercase name/path ascending, then insertion order so the
/// result matches the previous stable sort-then-truncate behavior exactly.
#[derive(Debug)]
struct ChildPruneKey {
    size: u64,
    name_key: String,
    path_key: String,
    seq: usize,
}

impl ChildPruneKey {
    fn compare(&self, other: &Self) -> Ordering {
        other
            .size
            .cmp(&self.size)
            .then_with(|| self.name_key.cmp(&other.name_key))
            .then_with(|| self.path_key.cmp(&other.path_key))
            .then_with(|| self.seq.cmp(&other.seq))
    }
}

enum ChildSource<'a> {
    /// A child directory node already built bottom-up (removed from `built`).
    Dir(AnalyzerNode),
    /// A retained file entry, turned into a node only if it survives pruning.
    File(&'a FileInfo),
}

/// A child candidate bundled with its precomputed prune key.
type ChildEntry<'a> = (ChildPruneKey, ChildSource<'a>);

/// Ranking for child candidates: the precomputed prune key decides order.
fn child_entry_compare(a: &ChildEntry<'_>, b: &ChildEntry<'_>) -> Ordering {
    a.0.compare(&b.0)
}

fn collect_directory_children(
    rec: &DirRecord,
    built: &mut HashMap<String, AnalyzerNode>,
    ctx: &AnnotationContext,
    settings: &EligibilitySettings,
    max_children: usize,
) -> Vec<AnalyzerNode> {
    let mut selected: BoundedTopN<ChildEntry<'_>> =
        BoundedTopN::new(max_children, child_entry_compare);
    let mut seq = 0usize;
    let mut seen_child = HashSet::new();

    for child_path in &rec.children_dirs {
        let child_key = canonical_key(child_path);
        if !seen_child.insert(child_key.clone()) {
            continue;
        }
        let Some(child_node) = built.remove(&child_key) else {
            continue;
        };
        let key = ChildPruneKey {
            size: child_node.size_bytes,
            name_key: child_node.name.to_ascii_lowercase(),
            path_key: child_node.path.to_ascii_lowercase(),
            seq,
        };
        seq += 1;
        selected.offer((key, ChildSource::Dir(child_node)));
    }

    for f in &rec.files {
        if f.name.is_empty() {
            continue;
        }
        let key = ChildPruneKey {
            size: f.size,
            name_key: f.name.to_ascii_lowercase(),
            path_key: f.path.to_ascii_lowercase(),
            seq,
        };
        seq += 1;
        selected.offer((key, ChildSource::File(f)));
    }

    selected
        .into_sorted()
        .into_iter()
        .map(|(_, source)| match source {
            ChildSource::Dir(node) => node,
            ChildSource::File(f) => build_file_node(f, ctx, settings),
        })
        .collect()
}

fn build_directory_node(
    rec: &DirRecord,
    size_bytes: u64,
    item_count: u64,
    children: Vec<AnalyzerNode>,
    ctx: &AnnotationContext,
    settings: &EligibilitySettings,
) -> AnalyzerNode {
    let ann = annotate_path(&rec.path, true, None, ctx, settings, Some(rec));
    AnalyzerNode {
        id: stable_node_id(&rec.path),
        path: rec.path.clone(),
        name: if rec.name.is_empty() {
            display_name(&rec.path)
        } else {
            rec.name.clone()
        },
        is_dir: true,
        size_bytes,
        item_count,
        risk: ann.risk,
        description: ann.description,
        tier: ann.tier,
        target_id: ann.target_id,
        is_link_point: ann.is_link_point,
        is_cloud_placeholder: ann.is_cloud_placeholder,
        display_only: ann.display_only,
        selectable: ann.selectable,
        selection_reason: ann.selection_reason,
        children,
    }
}

fn push_build_children(
    key: &str,
    records: &HashMap<String, DirRecord>,
    built: &HashMap<String, AnalyzerNode>,
    stack: &mut Vec<(String, bool, usize)>,
    depth: usize,
) {
    let Some(rec) = records.get(key) else {
        return;
    };
    let mut seen_child = HashSet::new();
    for child_path in &rec.children_dirs {
        let ck = canonical_key(child_path);
        if !seen_child.insert(ck.clone()) {
            continue;
        }
        if records.contains_key(&ck) && !built.contains_key(&ck) {
            stack.push((ck, false, depth + 1));
        }
    }
}

/// Builds the tree from an owned record map so each directory's source record
/// can be released as soon as its node is emitted. Aggregate totals are
/// computed up front from the complete map, so incremental release does not
/// change size/count values or the truncation warning.
pub(super) fn build_tree(
    mut records: HashMap<String, DirRecord>,
    root_key: &str,
    ctx: &AnnotationContext,
    settings: &EligibilitySettings,
    limits: TreeBuildLimits,
) -> Option<AnalyzerNode> {
    if !records.contains_key(root_key) {
        return None;
    }
    let totals = compute_directory_totals(&records, root_key);
    let mut built: HashMap<String, AnalyzerNode> = HashMap::new();
    let mut stack: Vec<(String, bool, usize)> = vec![(root_key.to_string(), false, 0)];
    let mut visited: HashSet<String> = HashSet::new();
    let max_children = limits.max_children_per_dir.max(1);
    let max_depth = limits.max_tree_depth;

    while let Some((key, expanded, depth)) = stack.pop() {
        if expanded {
            // Remove the record as its node is emitted so retained file
            // metadata for completed directories is released incrementally.
            let rec = records.remove(&key)?;
            let (size_bytes, item_count) = totals.get(&key).copied().unwrap_or((0, 0));
            let children = if depth < max_depth {
                collect_directory_children(&rec, &mut built, ctx, settings, max_children)
            } else {
                Vec::new()
            };
            let node = build_directory_node(&rec, size_bytes, item_count, children, ctx, settings);
            built.insert(key, node);
            continue;
        }
        if built.contains_key(&key) || !visited.insert(key.clone()) {
            continue;
        }
        stack.push((key.clone(), true, depth));
        if depth < max_depth {
            push_build_children(&key, &records, &built, &mut stack, depth);
        }
    }
    built.remove(root_key)
}

fn annotate_path(
    path_str: &str,
    is_dir: bool,
    file_info: Option<&FileInfo>,
    ctx: &AnnotationContext,
    settings: &EligibilitySettings,
    dir_record: Option<&DirRecord>,
) -> Annotation {
    let canon = CanonicalPath::parse_lexical(path_str).ok();
    let is_link_point = file_info.map(|f| f.is_link_point).unwrap_or(false)
        || dir_record
            .and_then(|r| r.files.iter().find(|f| f.name.is_empty()))
            .map(|m| m.is_link_point)
            .unwrap_or(false);
    let is_cloud_placeholder = file_info.map(|f| f.is_cloud_placeholder).unwrap_or(false)
        || dir_record
            .and_then(|r| r.files.iter().find(|f| f.name.is_empty()))
            .map(|m| m.is_cloud_placeholder)
            .unwrap_or(false);
    let reparse_other = file_info.map(|f| f.reparse_other).unwrap_or(false)
        || dir_record
            .and_then(|r| r.files.iter().find(|f| f.name.is_empty()))
            .map(|m| m.reparse_other)
            .unwrap_or(false);

    let is_vhdx = is_vhdx_path(path_str) || file_info.map(|f| f.is_vhdx).unwrap_or(false);
    let kb = find_knowledge_entry(path_str, canon.as_ref(), is_vhdx, ctx);
    let target = canon
        .as_ref()
        .and_then(|c| find_matching_target(c, &ctx.targets));

    let (risk, desc, tier, target_id) = classify_risk_and_description(
        path_str,
        canon.as_ref(),
        is_dir,
        is_vhdx,
        target,
        kb.as_ref(),
        ctx,
    );

    let protected = is_protected_for_analyzer(path_str, canon.as_ref(), risk);
    let display_only = is_vhdx
        || matches!(risk, Risk::Protected) && (protected || is_vhdx)
        || is_vhdx
        || kb
            .as_ref()
            .map(|k| is_display_only_note(&k.notes))
            .unwrap_or(false)
        || (is_vhdx && true);
    let display_only = display_only || is_vhdx;

    let mut selectable =
        !protected && !display_only && !is_link_point && !is_cloud_placeholder && !reparse_other;
    let mut reason = None;

    if protected {
        selectable = false;
        reason = Some("Protected path is not selectable".to_string());
    }
    if is_vhdx {
        selectable = false;
        reason = Some("WSL/Docker .vhdx files are display-only in v1".to_string());
    }
    if display_only && reason.is_none() {
        selectable = false;
        reason = Some("Display-only item is not selectable".to_string());
    }
    if is_link_point {
        selectable = false;
        reason = Some(
            "Link Point entries are never traversed or selected as content targets".to_string(),
        );
    }
    if is_cloud_placeholder {
        selectable = false;
        reason = Some("Cloud placeholders are never selected or hydrated".to_string());
    }
    if reparse_other {
        selectable = false;
        reason = Some("Non-link/unknown reparse points are skipped".to_string());
    }

    // Eligibility rules for Safe/Caution/Unknown (non-protected, non-display-only)
    if selectable {
        match risk {
            Risk::Safe => {
                // selectable
            }
            Risk::Caution => {
                if let Some(t) = tier {
                    if !settings.enabled_tiers.contains(&t) {
                        selectable = false;
                        reason = Some(format!("Caution tier {:?} is disabled in settings", t));
                    }
                } else {
                    // No tier context (e.g. knowledge-only caution entries) are not auto-selectable.
                    selectable = false;
                    reason = Some("Caution item has no enabled tier context".to_string());
                }
            }
            Risk::Unknown => {
                if !settings.allow_unknown {
                    selectable = false;
                    reason = Some("Unknown items require explicit opt-in".to_string());
                } else if !is_dir {
                    selectable = false;
                    reason = Some("Unknown files are not selectable; only eligible directories can be selected".to_string());
                } else {
                    let verdict = policy::validate_custom_path(path_str);
                    if !verdict.allowed {
                        selectable = false;
                        reason = Some(format!(
                            "Unknown directory is not eligible for selection: {}",
                            verdict.reason
                        ));
                    } else {
                        // Eligible Unknown directory: selectable as a Custom Path root (contents-only)
                        // and requires typed confirmation at conversion/clean time.
                        reason = None;
                    }
                }
            }
            Risk::Protected => {
                selectable = false;
                reason = Some("Protected items are not selectable".to_string());
            }
        }
    }

    let final_risk = if protected || is_protected_for_analyzer(path_str, canon.as_ref(), risk) {
        Risk::Protected
    } else if is_vhdx {
        Risk::Caution.max_for_display(risk)
    } else {
        risk
    };

    let (final_selectable, final_reason) = if final_risk == Risk::Protected || protected {
        (
            false,
            Some(reason.unwrap_or_else(|| "Protected path is not selectable".to_string())),
        )
    } else {
        (selectable, reason)
    };

    let description = if desc.is_empty() {
        default_description(path_str, is_dir, final_risk)
    } else {
        desc
    };

    Annotation {
        risk: final_risk,
        description,
        tier,
        target_id,
        is_link_point,
        is_cloud_placeholder,
        display_only: display_only || is_vhdx || final_risk == Risk::Protected && protected,
        selectable: final_selectable && !(display_only || is_vhdx),
        selection_reason: final_reason,
    }
}

trait RiskDisplayExt {
    fn max_for_display(self, other: Risk) -> Risk;
}
impl RiskDisplayExt for Risk {
    fn max_for_display(self, other: Risk) -> Risk {
        // For vhdx caution annotation preference when KB says caution; keep caution.
        if self == Risk::Caution || other == Risk::Caution {
            Risk::Caution
        } else {
            self
        }
    }
}

fn classify_risk_and_description(
    path_str: &str,
    canon: Option<&CanonicalPath>,
    _is_dir: bool,
    is_vhdx: bool,
    target: Option<&ResolvedTarget>,
    kb: Option<&KnowledgeEntry>,
    ctx: &AnnotationContext,
) -> (Risk, String, Option<Tier>, Option<String>) {
    if is_vhdx {
        if let Some(k) = kb {
            return (Risk::Caution, k.description.clone(), None, None);
        }
        return (
            Risk::Caution,
            "Virtual disk image (.vhdx). Display-only in Sweeper v1; never deleted or compacted."
                .to_string(),
            None,
            None,
        );
    }

    if let Some(t) = target {
        let desc = if t.def.description.is_empty() {
            format!("Allowlisted cleanup target: {}", t.def.name)
        } else {
            t.def.description.clone()
        };
        return (t.def.risk, desc, Some(t.def.tier), Some(t.def.id.clone()));
    }

    if is_protected_for_analyzer(path_str, canon, kb.map(|k| k.risk).unwrap_or(Risk::Unknown)) {
        if let Some(k) = kb {
            return (Risk::Protected, k.description.clone(), None, None);
        }
        return (
            Risk::Protected,
            "Protected path. Display-only and never selectable for Clean.".to_string(),
            None,
            None,
        );
    }

    // Protected container roots / non-allowlist descendants under them: protected for selection/display context.
    if is_protected_container_descendant(path_str, canon, ctx) {
        if let Some(k) = kb {
            return (Risk::Protected, k.description.clone(), None, None);
        }
        return (
            Risk::Protected,
            "Protected container or non-allowlisted descendant. Not selectable.".to_string(),
            None,
            None,
        );
    }

    if let Some(k) = kb {
        return (k.risk, k.description.clone(), None, None);
    }

    (
        Risk::Unknown,
        "Unknown location. Not recognized by built-in knowledge.".to_string(),
        None,
        None,
    )
}

fn is_protected_for_analyzer(
    path_str: &str,
    canon: Option<&CanonicalPath>,
    hinted_risk: Risk,
) -> bool {
    if hinted_risk == Risk::Protected {
        return true;
    }
    let Some(c) = canon else {
        // Conservative lexical fallback for obvious protected names
        let up = path_str.to_ascii_uppercase();
        return up.ends_with("\\SYSTEM32")
            || up.contains("\\WINDOWS\\")
            || up.contains("\\PROGRAM FILES")
            || up.contains("\\.GIT\\")
            || up.ends_with("\\.GIT");
    };
    policy::is_subtree_protected(c)
        || policy::is_ancestor_of_subtree_protected(c)
        || policy::is_protected_container_root(c)
}

fn is_protected_container_descendant(
    _path_str: &str,
    canon: Option<&CanonicalPath>,
    ctx: &AnnotationContext,
) -> bool {
    let Some(c) = canon else {
        return false;
    };
    if policy::is_protected_container_root(c) {
        return true;
    }
    if policy::is_under_protected_container_root(c) {
        // Allow only explicitly allowlisted subfolders under container roots; if it's not a resolved
        // allowlist target root or inside one, treat as protected/non-selectable for analyzer.
        if find_matching_target(c, &ctx.targets).is_some() {
            return false;
        }
        return true;
    }
    // Also treat descendants of drive root generically with caution? drive root itself is container root.
    false
}

fn find_matching_target<'a>(
    path: &CanonicalPath,
    targets: &'a [ResolvedTarget],
) -> Option<&'a ResolvedTarget> {
    // Prefer the most specific (deepest) matching root.
    let mut best: Option<&ResolvedTarget> = None;
    for t in targets {
        if t.root.is_self_or_ancestor_of(path) {
            match best {
                None => best = Some(t),
                Some(cur) if t.root.segment_count() > cur.root.segment_count() => best = Some(t),
                _ => {}
            }
        }
    }
    best
}

fn find_knowledge_entry(
    path_str: &str,
    canon: Option<&CanonicalPath>,
    is_vhdx: bool,
    ctx: &AnnotationContext,
) -> Option<KnowledgeEntry> {
    if is_vhdx {
        return ctx
            .known
            .iter()
            .find(|k| k.relative_pattern.contains(".vhdx") || k.id.contains("vhdx"))
            .cloned();
    }
    let name = canon
        .and_then(|c| c.segments().last())
        .map(|s| s.to_string())
        .or_else(|| {
            Path::new(path_str)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
        });
    let up = path_str.to_ascii_uppercase();

    // Specific exact/known mappings
    if up.ends_with("\\LOCAL\\PROGRAMS") || up.ends_with("\\APPDATA\\LOCAL\\PROGRAMS") {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-local-programs")
            .cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case(".rustup"))
        .unwrap_or(false)
    {
        return ctx.known.iter().find(|k| k.id == "kb-rustup").cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Packages"))
        .unwrap_or(false)
    {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-msix-packages")
            .cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Windows"))
        .unwrap_or(false)
        && up.ends_with(":\\WINDOWS")
    {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-windows-root")
            .cloned();
    }
    if up.ends_with(":\\USERS") {
        return ctx.known.iter().find(|k| k.id == "kb-users-root").cloned();
    }
    if let Some(profile) = &ctx.user_profile_root {
        if canon.map(|c| c.is_same_path(profile)).unwrap_or(false) {
            return ctx
                .known
                .iter()
                .find(|k| k.id == "kb-user-profile-root")
                .cloned();
        }
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Desktop"))
        .unwrap_or(false)
    {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-user-desktop")
            .cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Documents"))
        .unwrap_or(false)
    {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-user-documents")
            .cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Downloads"))
        .unwrap_or(false)
    {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-user-downloads")
            .cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Pictures"))
        .unwrap_or(false)
    {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-user-pictures")
            .cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Videos"))
        .unwrap_or(false)
    {
        return ctx.known.iter().find(|k| k.id == "kb-user-videos").cloned();
    }
    if name
        .as_deref()
        .map(|n| n.eq_ignore_ascii_case("Music"))
        .unwrap_or(false)
    {
        return ctx.known.iter().find(|k| k.id == "kb-user-music").cloned();
    }
    if up.ends_with(":\\PROGRAM FILES") {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-program-files")
            .cloned();
    }
    if up.ends_with(":\\PROGRAM FILES (X86)") {
        return ctx
            .known
            .iter()
            .find(|k| k.id == "kb-program-files-x86")
            .cloned();
    }
    None
}

fn is_display_only_note(notes: &str) -> bool {
    let n = notes.to_ascii_lowercase();
    n.contains("display-only")
        || n.contains("display only")
        || n.contains("never delete")
        || n.contains("never deleted")
}

fn default_description(path: &str, is_dir: bool, risk: Risk) -> String {
    let kind = if is_dir { "Folder" } else { "File" };
    match risk {
        Risk::Protected => format!("{kind} is protected and cannot be cleaned."),
        Risk::Unknown => format!("{kind} is not recognized by Sweeper's built-in knowledge base."),
        Risk::Caution => format!("{kind} may be recoverable but can be costly to regenerate."),
        Risk::Safe => {
            format!("{kind} is recognized as generally safe/recoverable in known contexts.")
        }
    }
    .to_string()
        + &format!(" ({})", path)
}

pub(super) fn build_annotation_context() -> AnnotationContext {
    let user_profile_root =
        sweeper_winfs::resolve_known_folder(sweeper_winfs::KnownFolderId::UserProfile)
            .ok()
            .and_then(|p| {
                CanonicalPath::from_existing_path(&p.to_string_lossy())
                    .ok()
                    .map(|v| v.into_canonical())
            });
    let targets = resolve_targets_for_annotation();
    AnnotationContext {
        user_profile_root,
        targets,
        known: sweeper_knowledge::builtin_knowledge_entries(),
    }
}

fn resolve_targets_for_annotation() -> Vec<ResolvedTarget> {
    let mut out = Vec::new();
    let defs = embedded_targets();
    let mut kf_cache: HashMap<sweeper_knowledge::KnownFolder, Option<CanonicalPath>> =
        HashMap::new();
    for def in defs {
        let base = kf_cache
            .entry(def.known_folder)
            .or_insert_with(|| resolve_known_folder_canonical(def.known_folder));
        let Some(base_root) = base.clone() else {
            continue;
        };
        for root in resolve_target_roots_for_def_with_base(&def, &base_root) {
            out.push(ResolvedTarget {
                def: def.clone(),
                root,
            });
        }
    }
    out
}

fn resolve_known_folder_canonical(kf: sweeper_knowledge::KnownFolder) -> Option<CanonicalPath> {
    let id = match kf {
        sweeper_knowledge::KnownFolder::LocalAppData => sweeper_winfs::KnownFolderId::LocalAppData,
        sweeper_knowledge::KnownFolder::RoamingAppData => {
            sweeper_winfs::KnownFolderId::RoamingAppData
        }
        sweeper_knowledge::KnownFolder::UserProfile => sweeper_winfs::KnownFolderId::UserProfile,
        sweeper_knowledge::KnownFolder::Windows => sweeper_winfs::KnownFolderId::Windows,
        sweeper_knowledge::KnownFolder::ProgramData => sweeper_winfs::KnownFolderId::ProgramData,
    };
    let p = sweeper_winfs::resolve_known_folder(id).ok()?;
    CanonicalPath::from_existing_path(&p.to_string_lossy())
        .ok()
        .map(|v| v.into_canonical())
        .or_else(|| CanonicalPath::parse_lexical(&p.to_string_lossy()).ok())
}

pub(super) fn resolve_target_roots_for_def(def: &TargetDefinition) -> Vec<CanonicalPath> {
    let Some(base) = resolve_known_folder_canonical(def.known_folder) else {
        return Vec::new();
    };
    resolve_target_roots_for_def_with_base(def, &base)
}

fn resolve_target_roots_for_def_with_base(
    def: &TargetDefinition,
    base_root: &CanonicalPath,
) -> Vec<CanonicalPath> {
    if def.relative_subpath.contains('*') {
        resolve_target_roots_wildcard(def, base_root)
    } else {
        resolve_target_root_single(def, base_root)
            .into_iter()
            .collect()
    }
}

fn resolve_target_root_single(
    def: &TargetDefinition,
    base_root: &CanonicalPath,
) -> Option<CanonicalPath> {
    if def.relative_subpath.contains('*') {
        return resolve_target_roots_wildcard(def, base_root)
            .into_iter()
            .next();
    }
    let mut path = PathBuf::from(base_root.as_str());
    for seg in def.relative_subpath.split('\\') {
        path.push(seg);
    }
    if !path.exists() {
        return None;
    }
    CanonicalPath::from_existing_path(&path.to_string_lossy())
        .ok()
        .map(|v| v.into_canonical())
        .or_else(|| CanonicalPath::parse_lexical(&path.to_string_lossy()).ok())
}

fn resolve_target_roots_wildcard(
    def: &TargetDefinition,
    base_root: &CanonicalPath,
) -> Vec<CanonicalPath> {
    let segments: Vec<&str> = def.relative_subpath.split('\\').collect();
    let mut candidates = vec![PathBuf::from(base_root.as_str())];
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
                    if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
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
        let canon = CanonicalPath::from_existing_path(&c.to_string_lossy())
            .ok()
            .map(|v| v.into_canonical())
            .or_else(|| CanonicalPath::parse_lexical(&c.to_string_lossy()).ok());
        let Some(canon) = canon else {
            continue;
        };
        if policy::is_subtree_protected(&canon)
            || policy::is_ancestor_of_subtree_protected(&canon)
            || policy::is_protected_container_root(&canon)
        {
            continue;
        }
        roots.push(canon);
    }
    roots.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    roots.dedup_by(|a, b| a.is_same_path(b));
    roots
}

pub(super) fn stable_node_id(path: &str) -> String {
    // Deterministic stable ID for a path (simple FNV-1a 64-bit hex), no extra deps.
    let key = canonical_key(path);
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("node-{hash:016x}")
}

pub(super) fn canonical_key(path: &str) -> String {
    let s = path.replace('/', "\\");
    let s = s
        .strip_prefix(r"\\?\UNC\")
        .map(|r| format!(r"\\{r}"))
        .unwrap_or_else(|| s.strip_prefix(r"\\?\").unwrap_or(&s).to_string());
    s.trim_end_matches('\\').to_ascii_uppercase()
}

pub(super) fn display_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    if trimmed.len() == 2 && trimmed.as_bytes().get(1) == Some(&b':') {
        return format!(r"{}\", &trimmed[0..1]);
    }
    if trimmed.len() == 3
        && trimmed.as_bytes().get(1) == Some(&b':')
        && (trimmed.ends_with('\\') || trimmed.ends_with('/'))
    {
        return trimmed.to_string();
    }
    Path::new(trimmed)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| trimmed.to_string())
}

pub(super) fn is_drive_root_input(raw: &str) -> bool {
    let s = raw.replace('/', "\\");
    (s.len() == 2 && s.as_bytes().get(1) == Some(&b':'))
        || (s.len() == 3 && s.as_bytes().get(1) == Some(&b':') && s.ends_with('\\'))
}

pub(super) fn is_vhdx_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("vhdx"))
        .unwrap_or(false)
}

/// Find a node by id in an analyzer tree.
pub fn find_node<'a>(root: &'a AnalyzerNode, id: &str) -> Option<&'a AnalyzerNode> {
    if root.id == id {
        return Some(root);
    }
    for child in &root.children {
        if let Some(found) = find_node(child, id) {
            return Some(found);
        }
    }
    None
}

/// Look up a target description by target id.
pub fn target_description(target_id: &str) -> Option<String> {
    embedded_targets()
        .into_iter()
        .find(|t| t.id == target_id)
        .map(|t| t.description)
}

/// Explain a known target or knowledge-base entry by id.
/// Returns `(title, description, risk)` when known.
pub fn knowledge_explanation(item_id: &str) -> Option<(String, String, Risk)> {
    if let Some(k) = sweeper_knowledge::builtin_knowledge_entries()
        .into_iter()
        .find(|k| k.id == item_id)
    {
        return Some((
            k.title,
            format!("{} {}", k.description, k.notes).trim().to_string(),
            k.risk,
        ));
    }
    if let Some(t) = embedded_targets().into_iter().find(|t| t.id == item_id) {
        return Some((t.name, t.description, t.risk));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::segment_pattern_matches;

    /// F6 regression: during `build_annotation_context`, a multi-byte directory
    /// name can satisfy the old byte-length guard while the slice offset falls
    /// mid-character. A verdict must be returned instead of a panic.
    #[test]
    fn multibyte_name_prefix_wildcard_returns_verdict_without_panic() {
        // "aééé" is 7 bytes; "Opera " is 6 bytes -> old name[..6] panicked.
        assert!(!segment_pattern_matches("a\u{e9}\u{e9}\u{e9}", "Opera *"));
        assert!(segment_pattern_matches(
            "Opera \u{e9}\u{e9}\u{e9}",
            "OPERA *"
        ));
    }
}
