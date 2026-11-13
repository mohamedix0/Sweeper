use super::*;
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
    pub(super) files: Vec<FileInfo>,
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
    pub(super) fn new(root_path: String) -> Self {
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
        }
    }
    pub(super) fn enqueue_initial(&self, dir_path: String) {
        let mut work = self.work.lock().unwrap();
        work.queue.push_back(dir_path);
        self.condvar.notify_all();
    }
    fn wait_for_work(&self, cancel: &AtomicBool) -> Option<String> {
        let mut work = self.work.lock().unwrap();
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
            work = self.condvar.wait(work).unwrap();
        }
    }
    fn finish_work(&self) {
        let mut work = self.work.lock().unwrap();
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
            let mut records = self.records.lock().unwrap();
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
                rec.scanned = true;
                if rec.name.is_empty() {
                    rec.name = annotate::display_name(dir_path);
                }
            }
        }
        if !traverse_children.is_empty() {
            let mut work = self.work.lock().unwrap();
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
        *self.current_path.lock().unwrap() = Some(dir_path.to_string());
    }
    pub(super) fn had_error(&self) -> bool {
        self.error.lock().unwrap().is_some()
    }
    pub(super) fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap().take()
    }
    pub(super) fn into_records(self: Arc<Self>) -> HashMap<String, DirRecord> {
        match Arc::try_unwrap(self) {
            Ok(s) => s.records.into_inner().unwrap_or_default(),
            Err(s) => s.records.lock().unwrap().clone(),
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
        let mut last = self.last_emit.lock().unwrap();
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
            current_path: state.current_path.lock().unwrap().clone(),
        }
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
        match scan_directory(&dir_path, &cancel) {
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
        state.finish_work();
        if cancel.load(AtomicOrdering::Relaxed) {
            state.condvar.notify_all();
            return;
        }
    }
}

fn scan_directory(dir_path: &str, cancel: &AtomicBool) -> Result<ScanDirOutput, String> {
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
    let mut files = Vec::new();
    let mut child_dirs = Vec::new();
    let mut file_count: u64 = 0;
    let mut byte_count: u64 = 0;
    for ent in entries {
        if cancel.load(AtomicOrdering::Relaxed) {
            break;
        }
        let name = ent.name.to_string_lossy().to_string();
        let path_str = ent.path.to_string_lossy().to_string();
        let is_vhdx = annotate::is_vhdx_path(&path_str);
        match ent.reparse {
            ReparseTagClass::LinkPoint => {
                if ent.is_dir {
                    child_dirs.push(ChildDirEntry {
                        path: path_str.clone(),
                        kind: ChildDirKind::Special(FileInfo {
                            name: String::new(),
                            path: path_str,
                            size: 0,
                            is_link_point: true,
                            is_cloud_placeholder: false,
                            reparse_other: false,
                            is_vhdx: false,
                        }),
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
                    file_count += 1;
                    byte_count = byte_count.saturating_add(ent.size);
                }
                continue;
            }
            ReparseTagClass::CloudPlaceholder => {
                if ent.is_dir {
                    child_dirs.push(ChildDirEntry {
                        path: path_str.clone(),
                        kind: ChildDirKind::Special(FileInfo {
                            name: String::new(),
                            path: path_str,
                            size: 0,
                            is_link_point: false,
                            is_cloud_placeholder: true,
                            reparse_other: false,
                            is_vhdx: false,
                        }),
                    });
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
                    file_count += 1;
                    byte_count = byte_count.saturating_add(ent.size);
                }
                continue;
            }
            ReparseTagClass::OtherReparse(_) => {
                if ent.is_dir {
                    child_dirs.push(ChildDirEntry {
                        path: path_str.clone(),
                        kind: ChildDirKind::Special(FileInfo {
                            name: String::new(),
                            path: path_str,
                            size: 0,
                            is_link_point: false,
                            is_cloud_placeholder: false,
                            reparse_other: true,
                            is_vhdx: false,
                        }),
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
                    file_count += 1;
                    byte_count = byte_count.saturating_add(ent.size);
                }
                continue;
            }
            ReparseTagClass::NotReparse => {}
        }
        if ent.is_dir {
            child_dirs.push(ChildDirEntry {
                path: path_str,
                kind: ChildDirKind::Traverse,
            });
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
            file_count += 1;
            byte_count = byte_count.saturating_add(ent.size);
        }
    }
    Ok(ScanDirOutput {
        files,
        child_dirs,
        file_count,
        byte_count,
    })
}
