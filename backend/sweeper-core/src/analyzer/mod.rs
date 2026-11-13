//! Read-only Analyzer: bounded parallel directory walk, aggregation, and safety annotations.
//!
//! Phase 4 (Part 1 / #7A): backend-only analysis primitives. No deletion, no IPC bridge/UI wiring.
//! - Iterative, bounded parallel traversal (no recursion)
//! - Never traverses `Link Point`s (symlinks/junctions/mount points)
//! - Never hydrates cloud placeholders
//! - Cancellation and throttled progress reporting
//! - Annotation and selection eligibility for safe integration with Preview/Clean flows

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use sweeper_knowledge::{Risk, Tier};

use crate::paths::{self, CanonicalPath};

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

impl Drop for AnalyzerNode {
    fn drop(&mut self) {
        let mut stack = std::mem::take(&mut self.children);
        while let Some(mut node) = stack.pop() {
            stack.append(&mut node.children);
        }
    }
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

    let lexical = if annotate::is_drive_root_input(raw) {
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
    let ctx = annotate::build_annotation_context();
    let settings = annotate::EligibilitySettings {
        allow_unknown: options.allow_unknown,
        ..annotate::EligibilitySettings::default()
    };

    let workers = walker::effective_worker_count(options.worker_threads);
    let state = Arc::new(walker::WalkState::new(root.as_str().to_string()));
    state.enqueue_initial(root.as_str().to_string());

    let progress_state = Arc::new(walker::ProgressState::new(
        progress,
        options.progress_interval,
    ));
    let mut handles = Vec::with_capacity(workers);

    for _ in 0..workers {
        let state_cloned = Arc::clone(&state);
        let cancel_cloned = Arc::clone(&cancel);
        let progress_cloned = Arc::clone(&progress_state);
        handles.push(std::thread::spawn(move || {
            walker::worker_loop(state_cloned, cancel_cloned, progress_cloned);
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    let cancelled = cancel.load(AtomicOrdering::Relaxed) || state.had_error();
    progress_state.emit_final(&state);

    if let Some(err) = state.take_error() {
        return Err(AnalyzerError::Walk(err));
    }

    let final_progress = progress_state.snapshot(&state);
    let records = state.into_records();
    let root_key = annotate::canonical_key(root.as_str());
    let tree = annotate::build_tree(&records, &root_key, &ctx, &settings)
        .ok_or_else(|| AnalyzerError::Walk("failed to build analyzer tree".to_string()))?;

    Ok(AnalyzerResult {
        root: tree,
        progress: final_progress,
        cancelled,
    })
}

mod annotate;
mod convert;
mod walker;

pub use annotate::{find_node as find_analyzer_node, knowledge_explanation, target_description};
pub use convert::{
    build_analyzer_preview_items, build_scan_items_from_analyzer, AnalyzerPreviewItem,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_root_input_normalization() {
        assert!(annotate::is_drive_root_input("C:"));
        assert!(annotate::is_drive_root_input(r"C:\"));
        assert!(annotate::is_drive_root_input("c:/"));
        assert!(!annotate::is_drive_root_input(r"C:\Temp"));
    }

    #[test]
    fn stable_node_id_is_deterministic_case_insensitive() {
        let a = annotate::stable_node_id(r"C:\Temp\Foo");
        let b = annotate::stable_node_id(r"c:\temp\foo");
        assert_eq!(a, b);
        assert!(a.starts_with("node-"));
    }
}
