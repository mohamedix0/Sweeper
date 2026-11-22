use super::bounded::BoundedTopN;
use super::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use sweeper_winfs::{self, ReparseTagClass};

#[derive(Debug, Clone)]
pub(super) struct FileInfo {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) is_link_point: bool,
    pub(super) is_cloud_placeholder: bool,
    pub(super) reparse_other: bool,
    pub(super) is_vhdx: bool,
}

#[derive(Debug)]
pub(super) struct DirRecord {
    pub(super) path: String,
    pub(super) name: String,
    pub(super) parent: Option<String>,
    pub(super) children_dirs: Vec<String>,
    /// Retained file entries for tree display (capped to top-N largest per directory).
    pub(super) files: Vec<FileInfo>,
    /// Exact count of all direct file entries observed in this directory (including retained and omitted).
    pub(super) own_file_count: u64,
    /// Exact logical bytes of all direct file entries observed in this directory.
    pub(super) own_bytes: u64,
    pub(super) scanned: bool,
}

impl DirRecord {
    pub(super) fn new(path: String, name: String, parent: Option<String>) -> Self {
        Self {
            path,
            name,
            parent,
            children_dirs: Vec::new(),
            files: Vec::new(),
            own_file_count: 0,
            own_bytes: 0,
            scanned: false,
        }
    }
}

pub(super) fn effective_worker_count(requested: usize) -> usize {
    if requested > 0 {
        return requested.clamp(1, 32);
    }
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    n.clamp(2, 8)
}

pub(super) struct WalkState {
    work: Mutex<WalkWork>,
    condvar: Condvar,
    records: Mutex<HashMap<String, DirRecord>>,
    error: Mutex<Option<String>>,
    scanned_dirs: AtomicU64,
    scanned_files: AtomicU64,
    scanned_bytes: AtomicU64,
    current_path: Mutex<Option<String>>,
    max_files_per_dir: usize,
}

#[derive(Debug, Default)]
struct WalkWork {
    queue: VecDeque<String>,
    active: usize,
    done: bool,
}

#[derive(Debug, Clone)]
enum ChildDirKind {
    Traverse,
    Special(FileInfo),
}

#[derive(Debug, Clone)]
struct ChildDirEntry {
    path: String,
    kind: ChildDirKind,
}

struct ScanDirOutput {
    files: Vec<FileInfo>,
    child_dirs: Vec<ChildDirEntry>,
    file_count: u64,
    byte_count: u64,
}

impl WalkState {
    pub(super) fn new(root_path: String, max_files_per_dir: usize) -> Self {
        let mut records = HashMap::new();
        let root_key = annotate::canonical_key(&root_path);
        let root_name = annotate::display_name(&root_path);
        records.insert(root_key, DirRecord::new(root_path, root_name, None));
        Self {
            work: Mutex::new(WalkWork::default()),
            condvar: Condvar::new(),
            records: Mutex::new(records),
            error: Mutex::new(None),
            scanned_dirs: AtomicU64::new(0),
            scanned_files: AtomicU64::new(0),
            scanned_bytes: AtomicU64::new(0),
            current_path: Mutex::new(None),
            max_files_per_dir: max_files_per_dir.max(1),
        }
    }
    pub(super) fn enqueue_initial(&self, dir_path: String) {
        let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        work.queue.push_back(dir_path);
        self.condvar.notify_all();
    }
    fn wait_for_work(&self, cancel: &AtomicBool) -> Option<String> {
        let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancel.load(AtomicOrdering::Relaxed) {
                self.condvar.notify_all();
                return None;
            }
            if let Some(p) = work.queue.pop_front() {
                work.active += 1;
                return Some(p);
            }
            if work.active == 0 {
                work.done = true;
                self.condvar.notify_all();
                return None;
            }
            work = self.condvar.wait(work).unwrap_or_else(|e| e.into_inner());
        }
    }
    fn finish_work(&self) {
        let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
        work.active = work.active.saturating_sub(1);
        if work.active == 0 && work.queue.is_empty() {
            work.done = true;
        }
        self.condvar.notify_all();
    }
    fn commit_directory_scan(
        &self,
        dir_path: &str,
        files: Vec<FileInfo>,
        child_dirs: Vec<ChildDirEntry>,
        file_count: u64,
        byte_count: u64,
    ) {
        let mut traverse_children = Vec::new();
        {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let dir_key = annotate::canonical_key(dir_path);
            records.entry(dir_key.clone()).or_insert_with(|| {
                DirRecord::new(dir_path.to_string(), annotate::display_name(dir_path), None)
            });
            let mut seen: HashSet<String> = HashSet::new();
            if let Some(parent) = records.get(&dir_key) {
                for c in &parent.children_dirs {
                    seen.insert(annotate::canonical_key(c));
                }
            }
            let mut new_children: Vec<String> = Vec::new();
            for child in child_dirs {
                let ck = annotate::canonical_key(&child.path);
                if seen.insert(ck.clone()) {
                    new_children.push(child.path.clone());
                }
                records.entry(ck.clone()).or_insert_with(|| {
                    DirRecord::new(
                        child.path.clone(),
                        annotate::display_name(&child.path),
                        Some(dir_path.to_string()),
                    )
                });
                if let Some(r) = records.get_mut(&ck) {
                    if r.parent.is_none() {
                        r.parent = Some(dir_path.to_string());
                    }
                    if r.name.is_empty() {
                        r.name = annotate::display_name(&child.path);
                    }
                    match child.kind {
                        ChildDirKind::Traverse => traverse_children.push(child.path),
                        ChildDirKind::Special(marker) => {
                            r.own_file_count = 0;
                            r.own_bytes = 0;
                            r.files = vec![marker];
                            r.children_dirs.clear();
                            r.scanned = true;
                        }
                    }
                }
            }
            if let Some(rec) = records.get_mut(&dir_key) {
                rec.children_dirs.extend(new_children);
                rec.files = files;
                rec.own_file_count = file_count;
                rec.own_bytes = byte_count;
                rec.scanned = true;
                if rec.name.is_empty() {
                    rec.name = annotate::display_name(dir_path);
                }
            }
        }
        if !traverse_children.is_empty() {
            let mut work = self.work.lock().unwrap_or_else(|e| e.into_inner());
            for c in traverse_children {
                work.queue.push_back(c);
            }
            self.condvar.notify_all();
        }
        self.scanned_dirs.fetch_add(1, AtomicOrdering::Relaxed);
        self.scanned_files
            .fetch_add(file_count, AtomicOrdering::Relaxed);
        self.scanned_bytes
            .fetch_add(byte_count, AtomicOrdering::Relaxed);
        *self.current_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir_path.to_string());
    }
    /// Records a walk failure so it surfaces as a typed `AnalyzerError::Walk`
    /// rather than being silently discarded. The first error wins.
    pub(super) fn record_error(&self, message: String) {
        let mut error = self.error.lock().unwrap_or_else(|e| e.into_inner());
        if error.is_none() {
            *error = Some(message);
        }
    }
    pub(super) fn had_error(&self) -> bool {
        self.error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    pub(super) fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
    pub(super) fn into_records(self: Arc<Self>) -> HashMap<String, DirRecord> {
        match Arc::try_unwrap(self) {
            Ok(s) => s.records.into_inner().unwrap_or_default(),
            Err(s) => s.records.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        }
    }
}

impl Clone for DirRecord {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            name: self.name.clone(),
            parent: self.parent.clone(),
            children_dirs: self.children_dirs.clone(),
            files: self.files.clone(),
            own_file_count: self.own_file_count,
            own_bytes: self.own_bytes,
            scanned: self.scanned,
        }
    }
}

pub(super) struct ProgressState {
    cb: Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>,
    interval: Duration,
    last_emit: Mutex<Instant>,
}
impl ProgressState {
    pub(super) fn new(
        cb: Option<Arc<dyn Fn(AnalyzerProgress) + Send + Sync>>,
        interval: Duration,
    ) -> Self {
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
        let mut last = self.last_emit.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= self.interval {
            *last = Instant::now();
            let snap = self.snapshot(state);
            drop(last);
            cb(snap);
        }
    }
    pub(super) fn emit_final(&self, state: &WalkState) {
        if let Some(cb) = &self.cb {
            cb(self.snapshot(state));
        }
    }
    pub(super) fn snapshot(&self, state: &WalkState) -> AnalyzerProgress {
        AnalyzerProgress {
            scanned_dirs: state.scanned_dirs.load(AtomicOrdering::Relaxed),
            scanned_files: state.scanned_files.load(AtomicOrdering::Relaxed),
            scanned_bytes: state.scanned_bytes.load(AtomicOrdering::Relaxed),
            current_path: state
                .current_path
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }
}

/// Releases a worker's reserved work slot when the loop exits, including via
/// panic unwinding. Without this guard a panicking worker leaves `active > 0`
/// forever, so peer workers blocked in `Condvar::wait` never observe `done`
/// and the join loop in `spawn_and_join_walkers` deadlocks (issue #134).
struct ActiveGuard<'a>(&'a WalkState);

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.finish_work();
    }
}

pub(super) fn worker_loop(
    state: Arc<WalkState>,
    cancel: Arc<AtomicBool>,
    progress: Arc<ProgressState>,
) {
    loop {
        if cancel.load(AtomicOrdering::Relaxed) {
            state.condvar.notify_all();
            return;
        }
        let Some(dir_path) = state.wait_for_work(&cancel) else {
            return;
        };
        // Registered immediately after the slot is reserved so any panic in
        // the scan/commit/progress path still releases the slot and wakes peers.
        let _active_guard = ActiveGuard(state.as_ref());
        match scan_directory(&dir_path, &cancel, state.max_files_per_dir) {
            Ok(out) => {
                state.commit_directory_scan(
                    &dir_path,
                    out.files,
                    out.child_dirs,
                    out.file_count,
                    out.byte_count,
                );
                progress.maybe_emit(&state);
            }
            Err(_) => {
                state.commit_directory_scan(&dir_path, Vec::new(), Vec::new(), 0, 0);
                progress.maybe_emit(&state);
            }
        }
        if cancel.load(AtomicOrdering::Relaxed) {
            state.condvar.notify_all();
            return;
        }
        // `_active_guard` is dropped here, releasing the work slot.
    }
}

/// A file paired with its precomputed prune key (lowercase path).
///
/// Keeping the key alongside the entry means ordering never re-lowercases a
/// path inside a comparator, and the buffer can stay bounded while scanning.
#[derive(Debug, Clone)]
struct KeyedFile {
    key: String,
    file: FileInfo,
}

/// Ranking for retained files: larger size first, then the precomputed key in
/// ascending order. Exact ties keep the earlier entry, matching the stable
/// ordering of the previous sort-then-truncate behaviour.
fn keyed_file_compare(a: &KeyedFile, b: &KeyedFile) -> Ordering {
    b.file
        .size
        .cmp(&a.file.size)
        .then_with(|| a.key.cmp(&b.key))
}

/// Return retained files in rank order, discarding the bundled prune keys.
fn into_sorted_files(files: BoundedTopN<KeyedFile>) -> Vec<FileInfo> {
    files
        .into_sorted()
        .into_iter()
        .map(|entry| entry.file)
        .collect()
}

#[derive(Debug)]
struct ScanAccum {
    top_files: BoundedTopN<KeyedFile>,
    child_dirs: Vec<ChildDirEntry>,
    file_count: u64,
    byte_count: u64,
}

impl ScanAccum {
    fn new(max_files: usize) -> Self {
        Self {
            top_files: BoundedTopN::new(max_files, keyed_file_compare),
            child_dirs: Vec::new(),
            file_count: 0,
            byte_count: 0,
        }
    }
}

fn reparse_flags(class: &ReparseTagClass) -> (bool, bool, bool) {
    match class {
        ReparseTagClass::LinkPoint => (true, false, false),
        ReparseTagClass::CloudPlaceholder => (false, true, false),
        ReparseTagClass::OtherReparse(_) => (false, false, true),
        ReparseTagClass::NotReparse => (false, false, false),
    }
}

fn push_special_dir(
    child_dirs: &mut Vec<ChildDirEntry>,
    path_str: String,
    flags: (bool, bool, bool),
) {
    child_dirs.push(ChildDirEntry {
        path: path_str.clone(),
        kind: ChildDirKind::Special(FileInfo {
            name: String::new(),
            path: path_str,
            size: 0,
            is_link_point: flags.0,
            is_cloud_placeholder: flags.1,
            reparse_other: flags.2,
            is_vhdx: false,
        }),
    });
}

fn push_file_entry(
    acc: &mut ScanAccum,
    name: String,
    path_str: String,
    size: u64,
    flags: (bool, bool, bool),
    is_vhdx: bool,
) {
    // Precompute the prune key exactly once and bundle it with the entry, so
    // ordering never re-lowercases a path and a huge directory stays bounded.
    let key = path_str.to_ascii_lowercase();
    acc.top_files.offer(KeyedFile {
        key,
        file: FileInfo {
            name,
            path: path_str,
            size,
            is_link_point: flags.0,
            is_cloud_placeholder: flags.1,
            reparse_other: flags.2,
            is_vhdx,
        },
    });
    acc.file_count += 1;
    acc.byte_count = acc.byte_count.saturating_add(size);
}

fn handle_reparse_entry(
    ent: &sweeper_winfs::DirEntry,
    name: String,
    path_str: String,
    is_vhdx: bool,
    acc: &mut ScanAccum,
) {
    let flags = reparse_flags(&ent.reparse);
    if ent.is_dir {
        push_special_dir(&mut acc.child_dirs, path_str, flags);
    } else {
        push_file_entry(acc, name, path_str, ent.size, flags, is_vhdx);
    }
}

fn scan_directory(
    dir_path: &str,
    cancel: &AtomicBool,
    max_files_per_dir: usize,
) -> Result<ScanDirOutput, String> {
    if cancel.load(AtomicOrdering::Relaxed) {
        return Ok(ScanDirOutput {
            files: Vec::new(),
            child_dirs: Vec::new(),
            file_count: 0,
            byte_count: 0,
        });
    }
    let entries =
        sweeper_winfs::read_dir_entries(Path::new(dir_path)).map_err(|e| e.to_string())?;
    let mut acc = ScanAccum::new(max_files_per_dir);
    for ent in entries {
        if cancel.load(AtomicOrdering::Relaxed) {
            break;
        }
        let name = ent.name.to_string_lossy().to_string();
        let path_str = ent.path.to_string_lossy().to_string();
        let is_vhdx = annotate::is_vhdx_path(&path_str);
        if !matches!(ent.reparse, ReparseTagClass::NotReparse) {
            handle_reparse_entry(&ent, name, path_str, is_vhdx, &mut acc);
            continue;
        }
        if ent.is_dir {
            acc.child_dirs.push(ChildDirEntry {
                path: path_str,
                kind: ChildDirKind::Traverse,
            });
        } else {
            push_file_entry(
                &mut acc,
                name,
                path_str,
                ent.size,
                (false, false, false),
                is_vhdx,
            );
        }
    }
    Ok(ScanDirOutput {
        files: into_sorted_files(acc.top_files),
        child_dirs: acc.child_dirs,
        file_count: acc.file_count,
        byte_count: acc.byte_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> FileInfo {
        FileInfo {
            name: name.to_string(),
            path: format!(r"C:\Temp\{name}"),
            size,
            is_link_point: false,
            is_cloud_placeholder: false,
            reparse_other: false,
            is_vhdx: false,
        }
    }

    #[test]
    fn bounded_top_files_never_retains_more_than_capacity() {
        let mut top = BoundedTopN::new(4, keyed_file_compare);
        for i in 0..5000u64 {
            let f = file(&format!("f{i:05}.bin"), i);
            top.offer(KeyedFile {
                key: f.path.to_ascii_lowercase(),
                file: f,
            });
            assert!(top.len() <= 4, "top-N buffer must stay bounded");
        }
        let files = into_sorted_files(top);
        assert_eq!(files.len(), 4);
        assert_eq!(
            files.iter().map(|f| f.size).collect::<Vec<_>>(),
            vec![4999, 4998, 4997, 4996]
        );
    }
}
