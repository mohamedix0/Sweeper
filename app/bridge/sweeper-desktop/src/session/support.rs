use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use sweeper_contract::ScanProgressEvent;
use sweeper_core::paths::CanonicalPath;
use sweeper_core::scanner::{self, ScanProgress};

pub(crate) fn sanitize_reason_pub(s: &str) -> String {
    sanitize_reason(s)
}

pub(crate) fn sanitize_reason(s: &str) -> String {
    let lower = s.to_lowercase();
    if lower.contains("path")
        || lower.contains('\\')
        || lower.contains('/')
        || lower.contains("stack")
    {
        "operation failed".to_string()
    } else {
        let mut out = s.trim().to_string();
        if out.len() > 120 {
            out.truncate(120);
        }
        if out.is_empty() {
            "operation failed".to_string()
        } else {
            out
        }
    }
}

pub(crate) fn emit_scan_progress<F>(shared: &Option<Arc<Mutex<F>>>, evt: ScanProgressEvent)
where
    F: FnMut(ScanProgressEvent),
{
    if let Some(cb) = shared {
        if let Ok(mut g) = cb.lock() {
            (*g)(evt);
        }
    }
}

pub(crate) fn build_store_scan_cb<F>(
    session_id: &str,
    shared: &Option<Arc<Mutex<F>>>,
) -> Option<scanner::ScanProgressCallback>
where
    F: FnMut(ScanProgressEvent) + Send + 'static,
{
    let cb = shared.as_ref()?.clone();
    let sid = session_id.to_string();
    Some(Arc::new(move |p: ScanProgress| {
        if let Ok(mut g) = cb.lock() {
            (*g)(ScanProgressEvent::Progress {
                session_id: sid.clone(),
                completed_targets: p.completed_targets,
                total_targets: p.total_targets,
                current_target_id: p.current_target_id,
                current_target_name: p.current_target_name,
                scanned_items: p.scanned_items.to_string(),
                scanned_bytes: p.scanned_bytes.to_string(),
            });
        }
    }) as scanner::ScanProgressCallback)
}

pub(crate) fn hash_selection(session_id: &str, item_ids: &[String]) -> String {
    let mut ids = item_ids.to_vec();
    ids.sort();
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update(b"\0");
    for id in ids {
        hasher.update(id.as_bytes());
        hasher.update(b"\n");
    }
    let out = hasher.finalize();
    hex::encode(out)
}

pub(crate) fn find_containing_root(roots: &[String], item_path: &str) -> Option<String> {
    let item = CanonicalPath::parse_lexical(item_path).ok()?;
    let mut best: Option<(usize, String)> = None;
    for root_str in roots {
        let Ok(root) = CanonicalPath::parse_lexical(root_str) else {
            continue;
        };
        if root.is_self_or_ancestor_of(&item) && !root.is_same_path(&item) {
            let depth = root.segment_count();
            match &best {
                Some((best_depth, _)) if *best_depth >= depth => {}
                _ => best = Some((depth, root_str.clone())),
            }
        }
    }
    best.map(|(_, r)| r)
}

pub(crate) fn list_log_ids(log_dir: &std::path::Path) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(log_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                out.insert(stem.to_string());
            }
        }
    }
    out
}

pub(crate) fn detect_new_log_id(
    log_dir: &std::path::Path,
    existing: &HashSet<String>,
) -> Option<String> {
    let mut candidates: Vec<(std::time::SystemTime, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(log_dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if existing.contains(stem) {
                continue;
            }
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            candidates.push((modified, stem.to_string()));
        }
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    candidates.into_iter().next().map(|(_, id)| id)
}
