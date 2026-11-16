use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use sweeper_contract::ScanProgressEvent;
use sweeper_core::paths::CanonicalPath;
use sweeper_core::scanner::{self, ScanProgress};

/// Wrapper to reduce string-arg ratio in public/internal helper signatures.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MsgRef<'a>(pub &'a str);

impl<'a> MsgRef<'a> {
    pub(crate) fn as_str(self) -> &'a str {
        self.0
    }
}

pub(crate) fn sanitize_reason_pub(msg: MsgRef<'_>) -> String {
    sanitize_reason(msg.as_str())
}

fn has_forbidden_reason_char(c: char) -> bool {
    matches!(c, '\\' | '/')
}

fn is_sensitive_reason(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    let has_path_word = lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| w == "path");
    let has_stack_word = lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| w == "stack");
    has_path_word || has_stack_word || msg.chars().any(has_forbidden_reason_char)
}

fn trim_reason(msg: &str) -> String {
    let mut out = msg.trim().to_string();
    if out.len() > 120 {
        let mut limit = 120;
        while limit > 0 && !out.is_char_boundary(limit) {
            limit -= 1;
        }
        out.truncate(limit);
    }
    if out.is_empty() {
        "operation failed".to_string()
    } else {
        out
    }
}

pub(crate) fn sanitize_reason(msg: &str) -> String {
    if is_sensitive_reason(msg) {
        "operation failed".to_string()
    } else {
        trim_reason(msg)
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

/// Session id wrapper to avoid string-heavy signatures.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SessionIdRef<'a>(pub &'a str);

pub(crate) fn build_store_scan_cb<F>(
    session_id: SessionIdRef<'_>,
    shared: &Option<Arc<Mutex<F>>>,
) -> Option<scanner::ScanProgressCallback>
where
    F: FnMut(ScanProgressEvent) + Send + 'static,
{
    let cb = shared.as_ref()?.clone();
    let sid = session_id.0.to_string();
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

pub(crate) fn hash_selection(session_id: SessionIdRef<'_>, item_ids: &[String]) -> String {
    let mut ids = item_ids.to_vec();
    ids.sort();
    let mut hasher = Sha256::new();
    hasher.update(session_id.0.as_bytes());
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

#[cfg(test)]
mod tests {
    use super::sanitize_reason;

    #[test]
    fn sanitize_reason_truncates_on_utf8_char_boundary() {
        let mut msg = "a".repeat(119);
        msg.push('é');
        let out = sanitize_reason(&msg);
        assert_eq!(out.chars().count(), 119);
        assert!(out.chars().all(|c| c == 'a'));
    }
}
