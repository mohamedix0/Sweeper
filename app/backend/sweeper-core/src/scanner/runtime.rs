use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(in crate::scanner) struct ScanProgressRuntime {
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) cb: Option<ScanProgressCallback>,
    pub(super) completed_targets: u32,
    pub(super) total_targets: u32,
    pub(super) scanned_items: u64,
    pub(super) scanned_bytes: u64,
    pub(super) last_emit_at: Instant,
    pub(super) last_emit_items: u64,
}

impl ScanProgressRuntime {
    pub(super) fn new(
        total_targets: u32,
        cancel: Arc<AtomicBool>,
        cb: Option<ScanProgressCallback>,
    ) -> Self {
        Self {
            cancel,
            cb,
            completed_targets: 0,
            total_targets,
            scanned_items: 0,
            scanned_bytes: 0,
            last_emit_at: Instant::now(),
            last_emit_items: 0,
        }
    }

    pub(super) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub(super) fn emit(
        &mut self,
        current_target_id: Option<&str>,
        current_target_name: Option<&str>,
    ) {
        if let Some(cb) = &self.cb {
            cb(ScanProgress {
                completed_targets: self.completed_targets,
                total_targets: self.total_targets,
                current_target_id: current_target_id.map(ToOwned::to_owned),
                current_target_name: current_target_name.map(ToOwned::to_owned),
                scanned_items: self.scanned_items,
                scanned_bytes: self.scanned_bytes,
            });
            self.last_emit_at = Instant::now();
            self.last_emit_items = self.scanned_items;
        }
    }

    pub(super) fn maybe_emit_throttled(
        &mut self,
        current_target_id: Option<&str>,
        current_target_name: Option<&str>,
    ) {
        if self.cb.is_none() {
            return;
        }
        let items_delta = self.scanned_items.saturating_sub(self.last_emit_items);
        if items_delta >= 64 || self.last_emit_at.elapsed() >= Duration::from_millis(100) {
            self.emit(current_target_id, current_target_name);
        }
    }
}
