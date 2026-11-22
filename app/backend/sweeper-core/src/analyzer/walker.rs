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
    /// Hard bound on the number of directory records retained for the whole walk.
    max_dir_records: usize,
    /// Hard bound on child-directory paths retained per directory record.
    max_child_dirs: usize,
    /// Set when either bound forces the walk to stop retaining/enqueuing directories.
    truncated: AtomicBool,
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
    pub(super) fn new(
        root_path: String,
        max_files_per_dir: usize,
        max_dir_records: usize,
        max_child_dirs: usize,
    ) -> Self {
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
            // The root record is inserted above, so both bounds are at least 1.
            max_dir_records: max_dir_records.max(1),
            max_child_dirs: max_child_dirs.max(1),
            truncated: AtomicBool::new(false),
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
    /// Dedup keys already linked to this parent, plus the number of them.
    fn existing_children(
        records: &HashMap<String, DirRecord>,
        dir_key: &str,
    ) -> (HashSet<String>, usize) {
        let mut seen = HashSet::new();
        let mut retained = 0usize;
        if let Some(parent) = records.get(dir_key) {
            for c in &parent.children_dirs {
                seen.insert(annotate::canonical_key(c));
            }
            retained = parent.children_dirs.len();
        }
        (seen, retained)
    }

    /// True when retaining this child would exceed the whole-walk record bound.
    fn dir_cap_reached(&self, records: &HashMap<String, DirRecord>, ck: &str) -> bool {
        !records.contains_key(ck) && records.len() >= self.max_dir_records
    }

    /// True when retaining another child would exceed this directory's child bound.
    fn child_cap_reached(&self, retained: usize, pending: usize) -> bool {
        retained + pending >= self.max_child_dirs
    }

    /// Insert/link one retained child and apply its traversal or special-marker semantics.
    fn retain_child(
        records: &mut HashMap<String, DirRecord>,
        child: ChildDirEntry,
        ck: String,
        dir_path: &str,
        traverse_children: &mut Vec<String>,
    ) {
        let path = child.path.clone();
        records.entry(ck.clone()).or_insert_with(|| {
            DirRecord::new(
                path.clone(),
                annotate::display_name(&path),
                Some(dir_path.to_string()),
            )
        });
        if let Some(r) = records.get_mut(&ck) {
            if r.parent.is_none() {
                r.parent = Some(dir_path.to_string());
            }
            if r.name.is_empty() {
                r.name = annotate::display_name(&path);
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

    /// Store the scan results on the directory record itself.
    fn finalize_parent(
        records: &mut HashMap<String, DirRecord>,
        dir_key: &str,
        dir_path: &str,
        new_children: Vec<String>,
        files: Vec<FileInfo>,
        file_count: u64,
        byte_count: u64,
    ) {
        if let Some(rec) = records.get_mut(dir_key) {
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

    fn enqueue_children(&self, traverse_children: Vec<String>) {
        if !traverse_children.is_empty() {
            let mut work = self.work.lock().unwrap();
            work.queue.extend(traverse_children);
            self.condvar.notify_all();
        }
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
        let mut did_truncate = false;
        {
            let mut records = self.records.lock().unwrap();
            let dir_key = annotate::canonical_key(dir_path);
            records.entry(dir_key.clone()).or_insert_with(|| {
                DirRecord::new(dir_path.to_string(), annotate::display_name(dir_path), None)
            });
            let (mut seen, retained_children) = Self::existing_children(&records, &dir_key);
            let mut new_children: Vec<String> = Vec::new();
            for child in child_dirs {
                let ck = annotate::canonical_key(&child.path);
                if !seen.insert(ck.clone()) {
                    continue;
                }
                // Bound retention *during* traversal: a skipped child gets no record,
                // no parent link and no queue entry, so the walk terminates early
                // instead of retaining one record per directory on the whole drive.
                if self.dir_cap_reached(&records, &ck)
                    || self.child_cap_reached(retained_children, new_children.len())
                {
                    did_truncate = true;
                    continue;
                }
                new_children.push(child.path.clone());
                Self::retain_child(&mut records, child, ck, dir_path, &mut traverse_children);
            }
            Self::finalize_parent(
                &mut records,
                &dir_key,
                dir_path,
                new_children,
                files,
                file_count,
                byte_count,
            );
        }
        if did_truncate {
            self.truncated.store(true, AtomicOrdering::Relaxed);
        }
        self.enqueue_children(traverse_children);
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
    pub(super) fn was_truncated(&self) -> bool {
        self.truncated.load(AtomicOrdering::Relaxed)
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
        state.finish_work();
        if cancel.load(AtomicOrdering::Relaxed) {
            state.condvar.notify_all();
            return;
        }
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
