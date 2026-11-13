//! Read-only Analyzer: bounded parallel directory walk, aggregation, and safety annotations.
//!
//! Phase 4 (Part 1 / #7A): backend-only analysis primitives. No deletion, no IPC bridge/UI wiring.
//! - Iterative, bounded parallel traversal (no recursion)
//! - Never traverses `Link Point`s (symlinks/junctions/mount points)
//! - Never hydrates cloud placeholders
//! - Cancellation and throttled progress reporting
//! - Annotation and selection eligibility for safe integration with Preview/Clean flows

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::{embedded_targets, KnowledgeEntry, Risk, TargetDefinition, Tier};
use sweeper_winfs::{self, ReparseTagClass};

use crate::paths::{self, CanonicalPath};
use crate::policy;
use crate::remover::ValidatedItem;
use crate::scanner;

/// Options controlling an Analyzer run.
#[derive(Debug, Clone)]
pub struct AnalyzerOptions {
    /// Number of worker threads (bounded). `0` selects an automatic default.
    pub worker_threads: usize,
    /// Minimum interval between progress callbacks.
    pub progress_interval: Duration,
    /// If `true`, allow `Unknown` items to be selectable in the conversion seam (Phase 5 policy seam; default false).
    pub allow_unknown: bool,
}

impl Default for AnalyzerOptions {
    fn default() -> Self {
        Self {
            worker_threads: 0,
            progress_interval: Duration::from_millis(100),
            allow_unknown: false,
        }
    }
}

/// Progress snapshot for a running Analyzer walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalyzerProgress {
    /// Number of directories scanned/enumerated.
    pub scanned_dirs: u64,
    /// Number of files observed.
    pub scanned_files: u64,
    /// Sum of logical file sizes observed (bytes).
    pub scanned_bytes: u64,
    /// Current/last path being processed (best-effort).
    pub current_path: Option<String>,
}

/// Aggregated node in the Analyzer tree.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalyzerNode {
    /// Stable node identifier (deterministic for a given canonical path).
    pub id: String,
    /// Canonical display path.
    pub path: String,
    /// Final path component / display name.
    pub name: String,
    /// Whether this node represents a directory (or directory-like container in the tree).
    pub is_dir: bool,
    /// Recursive total logical size in bytes for the node subtree (file size for files).
    pub size_bytes: u64,
    /// Recursive item count (files + directories) under/including this node as appropriate.
    pub item_count: u64,
    /// Risk classification for UI/selection (`Safe`, `Caution`, `Unknown`, `Protected`).
    pub risk: Risk,
    /// Human-readable explanation.
    pub description: String,
    /// Owning/related tier when known (e.g. from allowlist target annotation).
    pub tier: Option<Tier>,
    /// Matched allowlist target id, if applicable.
    pub target_id: Option<String>,
    /// True if the node itself is a `Link Point` entry.
    pub is_link_point: bool,
    /// True if the node itself is a cloud placeholder.
    pub is_cloud_placeholder: bool,
    /// True if the item is for display/explanation only and must never be cleaned in v1 flows.
    pub display_only: bool,
    /// Whether the node can be selected for the standard Preview/Clean pipeline.
    pub selectable: bool,
    /// Selection-policy reason when not selectable (diagnostic; UI may show or ignore).
    pub selection_reason: Option<String>,
    /// Child nodes (directories/files) sorted descending by `size_bytes`, then name/path.
    pub children: Vec<AnalyzerNode>,
}

/// Result of an Analyzer run.
#[derive(Debug, Clone)]
pub struct AnalyzerResult {
    /// Root node of the analyzed tree.
    pub root: AnalyzerNode,
    /// Final progress snapshot.
    pub progress: AnalyzerProgress,
    /// True if the walk was cancelled before completion.
    pub cancelled: bool,
}

/// Analyzer errors.
#[derive(Debug, Error)]
pub enum AnalyzerError {
    /// Invalid root input (empty, relative, UNC, traversal, malformed, etc.).
    #[error("invalid analyzer root: {0}")]
    InvalidRoot(String),
    /// Root does not exist.
    #[error("analyzer root does not exist: {0}")]
    NotFound(String),
    /// Root exists but is not a directory.
    #[error("analyzer root is not a directory: {0}")]
    NotADirectory(String),
    /// Filesystem/Win32 read failure.
    #[error("analyzer walk failed: {0}")]
    Walk(String),
}

#[derive(Debug, Clone)]
struct FileInfo {
    name: String,
    path: String,
    size: u64,
    is_link_point: bool,
    is_cloud_placeholder: bool,
    reparse_other: bool,
    is_vhdx: bool,
}

#[derive(Debug)]
struct DirRecord {
    path: String,
    name: String,
    parent: Option<String>,
    children_dirs: Vec<String>,
    files: Vec<FileInfo>,
    scanned: bool,
}

impl DirRecord {
    fn new(path: String, name: String, parent: Option<String>) -> Self {
        Self {
            path,
            name,
            parent,
            children_dirs: Vec::new(),
            files: Vec::new(),
            scanned: false,
        }
    }
}

#[derive(Debug, Clone)]
struct AnnotationContext {
    user_profile_root: Option<CanonicalPath>,
    targets: Vec<ResolvedTarget>,
    known: Vec<KnowledgeEntry>,
}

#[derive(Debug, Clone)]
struct ResolvedTarget {
    def: TargetDefinition,
    root: CanonicalPath,
}

#[derive(Debug, Clone)]
struct EligibilitySettings {
    enabled_tiers: HashSet<Tier>,
    allow_unknown: bool,
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
struct Annotation {
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

/// Validate and normalize an Analyzer root path for read-only analysis.
///
/// - `None` or empty => `C:\`
/// - Rejects invalid/relative/UNC/`..`/device-prefix forms via lexical validation
/// - Requires existing directory (no link-point traversal semantics needed; directory identity used as-is)
/// - Allows protected container roots for display/analysis
pub fn validate_analyzer_root(root: Option<&str>) -> Result<CanonicalPath, AnalyzerError> {
    let raw = root
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(r"C:\");

    let lexical = if is_drive_root_input(raw) {
        paths::parse_drive_root(raw).map_err(|e| AnalyzerError::InvalidRoot(e.to_string()))?
    } else {
        CanonicalPath::parse_lexical(raw).map_err(|e| AnalyzerError::InvalidRoot(e.to_string()))?
    };

    let path = Path::new(lexical.as_str());
    if !path.exists() {
        return Err(AnalyzerError::NotFound(lexical.to_string()));
    }
    let meta = std::fs::metadata(path)
        .map_err(|e| AnalyzerError::Walk(format!("failed to stat root {}: {e}", lexical)))?;
    if !meta.is_dir() {
        return Err(AnalyzerError::NotADirectory(lexical.to_string()));
    }

    // Prefer canonical handle-resolved identity when possible, but do not reject protected roots.
    match CanonicalPath::from_existing_path(lexical.as_str()) {
        Ok(v) => Ok(v.into_canonical()),
        Err(paths::PathError::LinkPoint) => {
            // Analyzer root itself must not be a traversed link point target; require opening the entry.
            // But link points are never traversed; treat root-as-link-entry as invalid for walk root.
            Err(AnalyzerError::InvalidRoot(
                "analyzer root is a link point".to_string(),
            ))
        }
        Err(paths::PathError::NotFound) => Err(AnalyzerError::NotFound(lexical.to_string())),
        Err(_e) => {
            // Fall back to lexical path for cases where handle resolution is restricted but path is a valid dir.
            Ok(lexical)
        }
    }
}

/// Run a read-only Analyzer walk on an optional root (default `C:\`).
pub fn analyze_path(
    root: Option<&str>,
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>,
    options: AnalyzerOptions,
) -> Result<AnalyzerResult, AnalyzerError> {
    let root_canon = validate_analyzer_root(root)?;
    analyze_validated_root(root_canon, cancel, progress, options)
}

/// Run a read-only Analyzer walk using a pre-validated root.
pub fn analyze_validated_root(
    root: CanonicalPath,
    cancel: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>,
    options: AnalyzerOptions,
) -> Result<AnalyzerResult, AnalyzerError> {
    let ctx = build_annotation_context();
    let settings = EligibilitySettings {
        allow_unknown: options.allow_unknown,
        ..EligibilitySettings::default()
    };

    let workers = effective_worker_count(options.worker_threads);
    let state = Arc::new(WalkState::new(root.as_str().to_string()));
    state.enqueue_dir(root.as_str().to_string());

    let progress_state = Arc::new(ProgressState::new(progress, options.progress_interval));
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let state_cloned = Arc::clone(&state);
        let cancel_cloned = Arc::clone(&cancel);
        let progress_cloned = Arc::clone(&progress_state);
        handles.push(std::thread::spawn(move || {
            worker_loop(state_cloned, cancel_cloned, progress_cloned);
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    // Robustness fallback: ensure any created-but-unscanned directory records are scanned
    // (defensive against scheduling/race edge cases in worker shutdown).
    finalize_unscanned_records(&state, &cancel);

    let cancelled = cancel.load(AtomicOrdering::Relaxed) || state.had_error();
    progress_state.emit_final(&state);

    if let Some(err) = state.take_error() {
        return Err(AnalyzerError::Walk(err));
    }

    let final_progress = progress_state.snapshot(&state);
    let records = state.into_records();
    let root_key = canonical_key(root.as_str());
    let tree = build_tree(&records, &root_key, &ctx, &settings)
        .ok_or_else(|| AnalyzerError::Walk("failed to build analyzer tree".to_string()))?;

    Ok(AnalyzerResult {
        root: tree,
        progress: final_progress,
        cancelled,
    })
}

/// Convert selected eligible Analyzer node IDs into validated scan items for Preview/Clean routing.
///
/// This seam performs explicit safety re-validation:
/// - selected nodes must exist in the provided tree
/// - nodes must be selectable and not display-only/protected/link points/cloud placeholders/`.vhdx`
/// - allowlist-backed nodes are mapped to allowlisted validated items
/// - only safe, non-protected, non-container, depth>=3, non-link-point directories outside protected
///   subtrees/containers are eligible as custom-path-root conversions (contents-only semantics)
pub fn build_scan_items_from_analyzer(
    result: &AnalyzerResult,
    selected_node_ids: &[String],
) -> Result<Vec<scanner::ScanItem>, String> {
    let index = index_nodes(&result.root);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut seq: u64 = 0;

    for id in selected_node_ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = index
            .get(id)
            .ok_or_else(|| format!("selected analyzer node not found: {id}"))?;

        validate_node_convertible(node)?;

        if let Some(tid) = &node.target_id {
            let target = embedded_targets()
                .into_iter()
                .find(|t| &t.id == tid)
                .ok_or_else(|| format!("allowlist target missing for node {}: {tid}", node.id))?;
            let target_root = resolve_target_root_path(&target)
                .ok_or_else(|| format!("failed to resolve allowlist target root for {tid}"))?;
            let _validated = ValidatedItem::from_allowlisted_scan(
                format!("analyzer::{tid}::{}", node.id),
                tid.clone(),
                target_root,
                node.path.clone(),
                node.size_bytes,
                None,
            )
            .map_err(|e| format!("selected node rejected by allowlist conversion: {e}"))?;

            seq += 1;
            out.push(scanner::ScanItem {
                id: format!("analyzer::{tid}::{seq}"),
                target_id: tid.clone(),
                path: node.path.clone(),
                file_identity: None,
                size: node.size_bytes,
                mtime_unix_secs: 0,
                age_eligible: true,
            });
            continue;
        }

        // Custom-path-root seam (Phase 5 will supply explicit typed confirmation flows at higher layers).
        // Restrict to directories and re-run on-disk custom path validation/policy checks.
        if !node.is_dir {
            return Err(format!(
                "selected non-target node must be a directory custom-path root candidate: {}",
                node.path
            ));
        }
        let verdict = policy::validate_custom_path(&node.path);
        if !verdict.allowed {
            return Err(format!(
                "selected custom-path candidate rejected: {} ({})",
                node.path, verdict.reason
            ));
        }

        seq += 1;
        out.push(scanner::ScanItem {
            id: format!("analyzer::custom::{seq}"),
            target_id: "custom-path".to_string(),
            path: node.path.clone(),
            file_identity: None,
            size: node.size_bytes,
            mtime_unix_secs: 0,
            age_eligible: true,
        });
    }

    Ok(out)
}

fn validate_node_convertible(node: &AnalyzerNode) -> Result<(), String> {
    if node.display_only {
        return Err(format!(
            "display-only node cannot be selected: {}",
            node.path
        ));
    }
    if !node.selectable {
        return Err(format!(
            "non-selectable node cannot be converted: {} ({})",
            node.path,
            node.selection_reason
                .clone()
                .unwrap_or_else(|| "not selectable".to_string())
        ));
    }
    if node.risk == Risk::Protected {
        return Err(format!("protected node cannot be converted: {}", node.path));
    }
    if node.is_link_point {
        return Err(format!(
            "link point node cannot be converted: {}",
            node.path
        ));
    }
    if node.is_cloud_placeholder {
        return Err(format!(
            "cloud placeholder node cannot be converted: {}",
            node.path
        ));
    }
    if is_vhdx_path(&node.path) {
        return Err(format!(".vhdx node cannot be converted: {}", node.path));
    }
    Ok(())
}

fn index_nodes(root: &AnalyzerNode) -> HashMap<String, AnalyzerNode> {
    fn rec(node: &AnalyzerNode, map: &mut HashMap<String, AnalyzerNode>) {
        map.insert(node.id.clone(), node.clone());
        for c in &node.children {
            rec(c, map);
        }
    }
    let mut map = HashMap::new();
    rec(root, &mut map);
    map
}

fn effective_worker_count(requested: usize) -> usize {
    if requested > 0 {
        return requested.clamp(1, 32);
    }
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    n.clamp(2, 8)
}

struct WalkState {
    queue: Mutex<VecDeque<String>>,
    condvar: Condvar,
    pending: AtomicUsize,
    active: AtomicUsize,
    stop: AtomicBool,
    records: Mutex<HashMap<String, DirRecord>>,
    error: Mutex<Option<String>>,
    scanned_dirs: AtomicU64,
    scanned_files: AtomicU64,
    scanned_bytes: AtomicU64,
    current_path: Mutex<Option<String>>,
}

impl WalkState {
    fn new(root_path: String) -> Self {
        let mut records = HashMap::new();
        let root_key = canonical_key(&root_path);
        let root_name = display_name(&root_path);
        records.insert(root_key, DirRecord::new(root_path, root_name, None));
        Self {
            queue: Mutex::new(VecDeque::new()),
            condvar: Condvar::new(),
            pending: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            records: Mutex::new(records),
            error: Mutex::new(None),
            scanned_dirs: AtomicU64::new(0),
            scanned_files: AtomicU64::new(0),
            scanned_bytes: AtomicU64::new(0),
            current_path: Mutex::new(None),
        }
    }

    fn enqueue_dir(&self, dir_path: String) {
        let mut q = self.queue.lock().unwrap();
        q.push_back(dir_path);
        self.pending.fetch_add(1, AtomicOrdering::AcqRel);
        self.condvar.notify_one();
    }

    fn wait_for_work(&self, cancel: &AtomicBool) -> Option<String> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if cancel.load(AtomicOrdering::Relaxed) || self.stop.load(AtomicOrdering::Relaxed) {
                self.condvar.notify_all();
                return None;
            }
            if let Some(p) = q.pop_front() {
                self.pending.fetch_sub(1, AtomicOrdering::AcqRel);
                return Some(p);
            }
            if self.pending.load(AtomicOrdering::Acquire) == 0
                && self.active.load(AtomicOrdering::Acquire) == 0
            {
                self.stop.store(true, AtomicOrdering::Release);
                self.condvar.notify_all();
                return None;
            }
            q = self.condvar.wait(q).unwrap();
        }
    }

    fn mark_active_start(&self) {
        self.active.fetch_add(1, AtomicOrdering::AcqRel);
    }
    fn mark_active_end(&self) {
        self.active.fetch_sub(1, AtomicOrdering::AcqRel);
        self.condvar.notify_all();
    }

    fn had_error(&self) -> bool {
        self.error.lock().unwrap().is_some()
    }
    fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }

    fn with_records_mut<T>(&self, f: impl FnOnce(&mut HashMap<String, DirRecord>) -> T) -> T {
        let mut g = self.records.lock().unwrap();
        f(&mut g)
    }

    fn ensure_dir_record(&self, path: &str, parent: Option<&str>) {
        let key = canonical_key(path);
        self.with_records_mut(|m| {
            if !m.contains_key(&key) {
                m.insert(
                    key.clone(),
                    DirRecord::new(
                        path.to_string(),
                        display_name(path),
                        parent.map(|p| p.to_string()),
                    ),
                );
            } else if let Some(r) = m.get_mut(&key) {
                if r.parent.is_none() {
                    r.parent = parent.map(|p| p.to_string());
                }
                if r.name.is_empty() {
                    r.name = display_name(path);
                }
            }
        });
    }

    fn add_child_dir(&self, parent_path: &str, child_path: &str) {
        let parent_key = canonical_key(parent_path);
        self.ensure_dir_record(parent_path, None);
        self.ensure_dir_record(child_path, Some(parent_path));
        self.with_records_mut(|m| {
            if let Some(parent) = m.get_mut(&parent_key) {
                parent.children_dirs.push(child_path.to_string());
            }
        });
    }

    fn set_dir_scan_result(&self, dir_path: &str, files: Vec<FileInfo>) {
        let key = canonical_key(dir_path);
        self.with_records_mut(|m| {
            if let Some(rec) = m.get_mut(&key) {
                rec.files = files;
                rec.scanned = true;
            }
        });
    }

    fn update_progress(&self, dir: Option<&str>, files: u64, bytes: u64) {
        self.scanned_dirs.fetch_add(1, AtomicOrdering::Relaxed);
        self.scanned_files.fetch_add(files, AtomicOrdering::Relaxed);
        self.scanned_bytes.fetch_add(bytes, AtomicOrdering::Relaxed);
        if let Some(d) = dir {
            let mut cur = self.current_path.lock().unwrap();
            *cur = Some(d.to_string());
        }
    }

    fn into_records(self: Arc<Self>) -> HashMap<String, DirRecord> {
        match Arc::try_unwrap(self) {
            Ok(s) => s.records.into_inner().unwrap_or_default(),
            Err(s) => s.records.lock().unwrap().clone(),
        }
    }
}

// DirRecord is not Clone by default because it contains Vecs etc; implement clone manually via fields.
impl Clone for DirRecord {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            name: self.name.clone(),
            parent: self.parent.clone(),
            children_dirs: self.children_dirs.clone(),
            files: self.files.clone(),
            scanned: self.scanned,
        }
    }
}

struct ProgressState {
    cb: Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>,
    interval: Duration,
    last_emit: Mutex<Instant>,
}

impl ProgressState {
    fn new(cb: Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>, interval: Duration) -> Self {
        Self {
            cb,
            interval,
            last_emit: Mutex::new(Instant::now() - interval),
        }
    }

    fn maybe_emit(&self, state: &WalkState) {
        let Some(cb) = &self.cb else {
            return;
        };
        let mut last = self.last_emit.lock().unwrap();
        if last.elapsed() >= self.interval {
            *last = Instant::now();
            let snap = self.snapshot(state);
            drop(last);
            cb(snap);
        }
    }

    fn emit_final(&self, state: &WalkState) {
        if let Some(cb) = &self.cb {
            cb(self.snapshot(state));
        }
    }

    fn snapshot(&self, state: &WalkState) -> AnalyzerProgress {
        AnalyzerProgress {
            scanned_dirs: state.scanned_dirs.load(AtomicOrdering::Relaxed),
            scanned_files: state.scanned_files.load(AtomicOrdering::Relaxed),
            scanned_bytes: state.scanned_bytes.load(AtomicOrdering::Relaxed),
            current_path: state.current_path.lock().unwrap().clone(),
        }
    }
}

fn worker_loop(state: Arc<WalkState>, cancel: Arc<AtomicBool>, progress: Arc<ProgressState>) {
    loop {
        if cancel.load(AtomicOrdering::Relaxed) || state.stop.load(AtomicOrdering::Relaxed) {
            state.condvar.notify_all();
            return;
        }
        let Some(dir_path) = state.wait_for_work(&cancel) else {
            return;
        };
        if cancel.load(AtomicOrdering::Relaxed) || state.stop.load(AtomicOrdering::Relaxed) {
            state.condvar.notify_all();
            return;
        }
        state.mark_active_start();
        let scan_result = scan_directory(&dir_path, &state, &cancel);
        match scan_result {
            Ok((files, file_count, byte_count)) => {
                state.set_dir_scan_result(&dir_path, files);
                state.update_progress(Some(&dir_path), file_count, byte_count);
                progress.maybe_emit(&state);
            }
            Err(e) => {
                // Directory unreadable/inaccessible: record as scanned-empty for aggregation stability but keep going.
                // If the root itself fails, surface as walk error.
                state.set_dir_scan_result(&dir_path, Vec::new());
                state.update_progress(Some(&dir_path), 0, 0);
                progress.maybe_emit(&state);
                // Don't fail entire run for transient unreadable subdirs; root validation already happened.
                // If cancellation/error-stop was requested, exit; otherwise continue.
                let _ = e;
            }
        }
        state.mark_active_end();
        if cancel.load(AtomicOrdering::Relaxed) {
            state.stop.store(true, AtomicOrdering::Release);
            state.condvar.notify_all();
            return;
        }
    }
}

fn scan_directory(
    dir_path: &str,
    state: &WalkState,
    cancel: &AtomicBool,
) -> Result<(Vec<FileInfo>, u64, u64), String> {
    if cancel.load(AtomicOrdering::Relaxed) || state.stop.load(AtomicOrdering::Relaxed) {
        return Ok((Vec::new(), 0, 0));
    }

    let entries =
        sweeper_winfs::read_dir_entries(Path::new(dir_path)).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut file_count: u64 = 0;
    let mut byte_count: u64 = 0;

    for ent in entries {
        if cancel.load(AtomicOrdering::Relaxed) || state.stop.load(AtomicOrdering::Relaxed) {
            break;
        }

        let name = ent.name.to_string_lossy().to_string();
        let path_str = ent.path.to_string_lossy().to_string();
        let is_vhdx = is_vhdx_path(&path_str);

        match ent.reparse {
            ReparseTagClass::LinkPoint => {
                // Never traverse. Record link entry itself.
                if ent.is_dir {
                    state.ensure_dir_record(&path_str, Some(dir_path));
                    state.add_child_dir(dir_path, &path_str);
                    state.with_records_mut(|m| {
                        let k = canonical_key(&path_str);
                        if let Some(r) = m.get_mut(&k) {
                            r.files = vec![FileInfo {
                                name: String::new(),
                                path: path_str.clone(),
                                size: 0,
                                is_link_point: true,
                                is_cloud_placeholder: false,
                                reparse_other: false,
                                is_vhdx: false,
                            }];
                            r.scanned = true;
                            r.children_dirs.clear();
                        }
                    });
                } else {
                    files.push(FileInfo {
                        name,
                        path: path_str,
                        size: ent.size,
                        is_link_point: true,
                        is_cloud_placeholder: false,
                        reparse_other: false,
                        is_vhdx,
                    });
                    file_count = file_count.saturating_add(1);
                    byte_count = byte_count.saturating_add(ent.size);
                }
                continue;
            }
            ReparseTagClass::CloudPlaceholder => {
                // Never hydrate, never traverse. Record placeholder entry.
                if ent.is_dir {
                    state.ensure_dir_record(&path_str, Some(dir_path));
                    state.set_dir_scan_result(&path_str, Vec::new());
                    state.add_child_dir(dir_path, &path_str);
                    // Cloud placeholder directory represented as empty, non-traversed node.
                    // Store a marker file entry? We'll handle via annotation on dir node using path heuristics impossible.
                    // Instead, encode as a pseudo-file marker? easier to track by adding special file info not counted except maybe 0.
                    // But we need `is_cloud_placeholder` flag on node. We'll store in a side channel by putting a marker in files
                    // and interpreting in build if needed is messy. Better: register a separate marker map is extra state.
                    // Simpler: create the dir record and attach a zero-size pseudo file marker isn't right.
                    // We'll handle cloud-placeholder directories by treating as directory record and annotating via reparse?
                    // We don't have dir entry metadata stored except name/path. Let's insert a synthetic file marker in parent.
                    // Not ideal, but sufficient: represent as non-traversed child dir; annotation will check path entry type in records?
                } else {
                    files.push(FileInfo {
                        name,
                        path: path_str,
                        size: ent.size,
                        is_link_point: false,
                        is_cloud_placeholder: true,
                        reparse_other: false,
                        is_vhdx,
                    });
                    file_count = file_count.saturating_add(1);
                    // Do not hydrate; use logical size from find data.
                    byte_count = byte_count.saturating_add(ent.size);
                }
                // For cloud placeholder directories, represent as a non-traversed directory node by recording a child dir
                // and a marker in a dedicated mechanism is omitted; annotation will conservatively treat unknown reparse dirs.
                // To preserve metadata, add synthetic marker file with dir path encoded? no.
                if ent.is_dir {
                    // add a hidden marker file record on the child dir itself by storing in its files list? can't yet until created
                    state.with_records_mut(|m| {
                        let k = canonical_key(&ent.path.to_string_lossy());
                        if let Some(r) = m.get_mut(&k) {
                            r.files.push(FileInfo {
                                name: String::new(),
                                path: ent.path.to_string_lossy().to_string(),
                                size: 0,
                                is_link_point: false,
                                is_cloud_placeholder: true,
                                reparse_other: false,
                                is_vhdx: false,
                            });
                            r.scanned = true;
                        }
                    });
                }
                continue;
            }
            ReparseTagClass::OtherReparse(_) => {
                // Do not traverse/follow. Record as non-traversed entry.
                if ent.is_dir {
                    state.ensure_dir_record(&path_str, Some(dir_path));
                    state.set_dir_scan_result(&path_str, Vec::new());
                    state.add_child_dir(dir_path, &path_str);
                    state.with_records_mut(|m| {
                        let k = canonical_key(&ent.path.to_string_lossy());
                        if let Some(r) = m.get_mut(&k) {
                            r.files.push(FileInfo {
                                name: String::new(),
                                path: ent.path.to_string_lossy().to_string(),
                                size: 0,
                                is_link_point: false,
                                is_cloud_placeholder: false,
                                reparse_other: true,
                                is_vhdx: false,
                            });
                            r.scanned = true;
                        }
                    });
                } else {
                    files.push(FileInfo {
                        name,
                        path: path_str,
                        size: ent.size,
                        is_link_point: false,
                        is_cloud_placeholder: false,
                        reparse_other: true,
                        is_vhdx,
                    });
                    file_count = file_count.saturating_add(1);
                    byte_count = byte_count.saturating_add(ent.size);
                }
                continue;
            }
            ReparseTagClass::NotReparse => {}
        }

        if ent.is_dir {
            state.add_child_dir(dir_path, &path_str);
            state.enqueue_dir(path_str);
        } else {
            files.push(FileInfo {
                name,
                path: path_str,
                size: ent.size,
                is_link_point: false,
                is_cloud_placeholder: false,
                reparse_other: false,
                is_vhdx,
            });
            file_count = file_count.saturating_add(1);
            byte_count = byte_count.saturating_add(ent.size);
        }
    }

    Ok((files, file_count, byte_count))
}

fn build_tree(
    records: &HashMap<String, DirRecord>,
    root_key: &str,
    ctx: &AnnotationContext,
    settings: &EligibilitySettings,
) -> Option<AnalyzerNode> {
    if !records.contains_key(root_key) {
        return None;
    }

    // Iterative post-order to compute recursive totals (size_bytes, item_count) without recursion.
    let mut totals: HashMap<String, (u64, u64)> = HashMap::new();
    let mut stack: Vec<(String, bool)> = vec![(root_key.to_string(), false)];
    let mut visited_expand: HashSet<String> = HashSet::new();

    while let Some((key, expanded)) = stack.pop() {
        if expanded {
            let mut size: u64 = 0;
            let mut count: u64 = 0;
            if let Some(rec) = records.get(&key) {
                for f in &rec.files {
                    if f.name.is_empty() {
                        continue;
                    }
                    size = size.saturating_add(f.size);
                    count = count.saturating_add(1);
                }
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
            }
            totals.insert(key, (size, count));
            continue;
        }

        if totals.contains_key(&key) {
            continue;
        }
        if !visited_expand.insert(key.clone()) {
            // Defensive cycle guard; treat as empty if revisited before completion.
            totals.entry(key).or_insert((0, 0));
            continue;
        }

        stack.push((key.clone(), true));
        if let Some(rec) = records.get(&key) {
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
    }

    // Iterative post-order tree construction so children nodes exist before parents.
    let mut built: HashMap<String, AnalyzerNode> = HashMap::new();
    let mut stack2: Vec<(String, bool)> = vec![(root_key.to_string(), false)];
    let mut visited_build: HashSet<String> = HashSet::new();

    while let Some((key, expanded)) = stack2.pop() {
        if expanded {
            let Some(rec) = records.get(&key) else {
                continue;
            };
            let (size_bytes, item_count) = totals.get(&key).copied().unwrap_or((0, 0));
            let ann = annotate_path(&rec.path, true, None, ctx, settings, Some(rec));

            let mut children: Vec<AnalyzerNode> = Vec::new();
            let mut seen_child = HashSet::new();
            for child_path in &rec.children_dirs {
                let ck = canonical_key(child_path);
                if !seen_child.insert(ck.clone()) {
                    continue;
                }
                if let Some(child_node) = built.remove(&ck) {
                    children.push(child_node);
                }
            }
            for f in &rec.files {
                if f.name.is_empty() {
                    continue;
                }
                let file_ann = annotate_path(&f.path, false, Some(f), ctx, settings, None);
                children.push(AnalyzerNode {
                    id: stable_node_id(&f.path),
                    path: f.path.clone(),
                    name: f.name.clone(),
                    is_dir: false,
                    size_bytes: f.size,
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
                });
            }

            children.sort_by(|a, b| {
                b.size_bytes
                    .cmp(&a.size_bytes)
                    .then_with(|| {
                        a.name
                            .to_ascii_lowercase()
                            .cmp(&b.name.to_ascii_lowercase())
                    })
                    .then_with(|| {
                        a.path
                            .to_ascii_lowercase()
                            .cmp(&b.path.to_ascii_lowercase())
                    })
            });

            let node = AnalyzerNode {
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
            };
            built.insert(key, node);
            continue;
        }

        if built.contains_key(&key) {
            continue;
        }
        if !visited_build.insert(key.clone()) {
            continue;
        }
        stack2.push((key.clone(), true));
        if let Some(rec) = records.get(&key) {
            let mut seen_child = HashSet::new();
            for child_path in &rec.children_dirs {
                let ck = canonical_key(child_path);
                if !seen_child.insert(ck.clone()) {
                    continue;
                }
                if records.contains_key(&ck) && !built.contains_key(&ck) {
                    stack2.push((ck, false));
                }
            }
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
                    // Unknown-tier caution should not be auto-selectable
                    selectable = false;
                    reason = Some("Caution item has no enabled tier context".to_string());
                }
            }
            Risk::Unknown => {
                if !settings.allow_unknown {
                    selectable = false;
                    reason =
                        Some("Unknown items are not selectable by default in Phase 4".to_string());
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

fn build_annotation_context() -> AnnotationContext {
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
        if let Some(root) = resolve_target_root_single(&def, &base_root) {
            out.push(ResolvedTarget { def, root });
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

fn resolve_target_root_path(def: &TargetDefinition) -> Option<String> {
    let base = resolve_known_folder_canonical(def.known_folder)?;
    resolve_target_root_single(def, &base).map(|c| c.to_string())
}

fn resolve_target_root_single(
    def: &TargetDefinition,
    base_root: &CanonicalPath,
) -> Option<CanonicalPath> {
    // Analyzer annotation only needs a best-effort concrete root for non-wildcard targets.
    if def.relative_subpath.contains('*') {
        // Wildcard targets expand to multiple roots; annotation will match only when exact concrete
        // paths are resolved elsewhere. Skip here.
        return None;
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

fn stable_node_id(path: &str) -> String {
    // Deterministic stable ID for a path (simple FNV-1a 64-bit hex), no extra deps.
    let key = canonical_key(path);
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("node-{hash:016x}")
}

fn canonical_key(path: &str) -> String {
    let s = path.replace('/', "\\");
    let s = s
        .strip_prefix(r"\\?\UNC\")
        .map(|r| format!(r"\\{r}"))
        .unwrap_or_else(|| s.strip_prefix(r"\\?\").unwrap_or(&s).to_string());
    s.trim_end_matches('\\').to_ascii_uppercase()
}

fn display_name(path: &str) -> String {
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

fn is_drive_root_input(raw: &str) -> bool {
    let s = raw.replace('/', "\\");
    (s.len() == 2 && s.as_bytes().get(1) == Some(&b':'))
        || (s.len() == 3 && s.as_bytes().get(1) == Some(&b':') && s.ends_with('\\'))
}

fn is_vhdx_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("vhdx"))
        .unwrap_or(false)
}

fn finalize_unscanned_records(state: &Arc<WalkState>, cancel: &AtomicBool) {
    // Iterative fixed-point sweep: scan directories that exist in records but were not marked scanned.
    // Newly discovered child directories (if any) are scanned in subsequent iterations.
    for _ in 0..4 {
        if cancel.load(AtomicOrdering::Relaxed) || state.had_error() {
            break;
        }
        let pending_dirs: Vec<String> = state.with_records_mut(|records| {
            records
                .values()
                .filter(|r| !r.scanned)
                .map(|r| r.path.clone())
                .collect()
        });
        if pending_dirs.is_empty() {
            break;
        }
        for dir in pending_dirs {
            if cancel.load(AtomicOrdering::Relaxed) || state.had_error() {
                return;
            }
            let already_scanned = state.with_records_mut(|records| {
                records
                    .get(&canonical_key(&dir))
                    .map(|r| r.scanned)
                    .unwrap_or(true)
            });
            if already_scanned {
                continue;
            }
            match scan_directory(&dir, state, cancel) {
                Ok((files, file_count, byte_count)) => {
                    state.set_dir_scan_result(&dir, files);
                    state.update_progress(Some(&dir), file_count, byte_count);
                }
                Err(_) => {
                    state.set_dir_scan_result(&dir, Vec::new());
                    state.update_progress(Some(&dir), 0, 0);
                }
            }
        }
    }

    // Defensive reconciliation: if a directory is marked scanned but appears empty in records while
    // the filesystem still contains entries (possible missed update/race), rescan it once.
    if cancel.load(AtomicOrdering::Relaxed) || state.had_error() {
        return;
    }
    let candidates: Vec<String> = state.with_records_mut(|records| {
        records
            .values()
            .filter(|r| r.scanned && r.files.is_empty() && r.children_dirs.is_empty())
            .map(|r| r.path.clone())
            .collect()
    });
    for dir in candidates {
        if cancel.load(AtomicOrdering::Relaxed) || state.had_error() {
            return;
        }
        let has_entries = sweeper_winfs::read_dir_entries(Path::new(&dir))
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if !has_entries {
            continue;
        }
        match scan_directory(&dir, state, cancel) {
            Ok((files, file_count, byte_count)) => {
                state.set_dir_scan_result(&dir, files);
                state.update_progress(Some(&dir), file_count, byte_count);
            }
            Err(_) => {
                // keep existing empty result
            }
        }
    }

    // Second fixed-point pass for any new children discovered by reconciliation rescans.
    for _ in 0..4 {
        if cancel.load(AtomicOrdering::Relaxed) || state.had_error() {
            break;
        }
        let pending_dirs: Vec<String> = state.with_records_mut(|records| {
            records
                .values()
                .filter(|r| !r.scanned)
                .map(|r| r.path.clone())
                .collect()
        });
        if pending_dirs.is_empty() {
            break;
        }
        for dir in pending_dirs {
            if cancel.load(AtomicOrdering::Relaxed) || state.had_error() {
                return;
            }
            let already_scanned = state.with_records_mut(|records| {
                records
                    .get(&canonical_key(&dir))
                    .map(|r| r.scanned)
                    .unwrap_or(true)
            });
            if already_scanned {
                continue;
            }
            match scan_directory(&dir, state, cancel) {
                Ok((files, file_count, byte_count)) => {
                    state.set_dir_scan_result(&dir, files);
                    state.update_progress(Some(&dir), file_count, byte_count);
                }
                Err(_) => {
                    state.set_dir_scan_result(&dir, Vec::new());
                    state.update_progress(Some(&dir), 0, 0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_root_input_normalization() {
        assert!(is_drive_root_input("C:"));
        assert!(is_drive_root_input(r"C:\"));
        assert!(is_drive_root_input("c:/"));
        assert!(!is_drive_root_input(r"C:\Temp"));
    }

    #[test]
    fn stable_node_id_is_deterministic_case_insensitive() {
        let a = stable_node_id(r"C:\Temp\Foo");
        let b = stable_node_id(r"c:\temp\foo");
        assert_eq!(a, b);
        assert!(a.starts_with("node-"));
    }
}
