#![cfg(windows)]

//! Regression test for issue #134 (audit finding C4).
//!
//! A worker panic must be surfaced as a typed `AnalyzerError::Walk` instead of
//! being discarded by `let _ = h.join()` and silently returning a truncated
//! tree with `cancelled = false`. The multi-worker case also proves that a
//! panicking worker releases its work slot so peers are not left deadlocked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use sweeper_core::analyzer::{
    analyze_path, AnalyzerError, AnalyzerOptions, AnalyzerProgress, AnalyzerResult,
};

/// Unique payload text asserted end-to-end; the constant error prefix alone
/// ("analyzer worker panicked: ") must not satisfy the assertion.
const PANIC_MARKER: &str = "simulated analyzer worker panic";

/// Runs a walk whose progress callback panics exactly once inside a worker.
///
/// The walk runs on a helper thread with a watchdog so a deadlock (e.g. a
/// panicking worker leaving its reserved work slot held) fails fast instead of
/// hanging the test suite.
fn analyze_with_panicking_worker(
    worker_threads: usize,
) -> (bool, Result<AnalyzerResult, AnalyzerError>) {
    // Machine-neutral, real directory so root validation succeeds without
    // touching any real user/system path. Kept alive for the whole walk.
    let root = tempfile::tempdir().expect("create temp root");
    let root_str = root.path().to_string_lossy().to_string();

    let panicked = Arc::new(AtomicBool::new(false));
    let panicked_in_cb = Arc::clone(&panicked);
    let progress: Arc<dyn Fn(AnalyzerProgress) + Send + Sync> = Arc::new(move |_| {
        if !panicked_in_cb.swap(true, Ordering::SeqCst) {
            panic!("{PANIC_MARKER} (issue #134)");
        }
    });

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = analyze_path(
            Some(&root_str),
            Arc::new(AtomicBool::new(false)),
            Some(progress),
            AnalyzerOptions {
                worker_threads,
                progress_interval: Duration::ZERO,
                ..AnalyzerOptions::default()
            },
        );
        let _ = tx.send(result);
    });

    let result = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("analyzer walk deadlocked after a worker panic");
    (panicked.load(Ordering::SeqCst), result)
}

fn assert_typed_walk_error(worker_threads: usize) {
    let (panicked, result) = analyze_with_panicking_worker(worker_threads);
    assert!(
        panicked,
        "worker_threads={worker_threads}: progress callback should have panicked in a worker"
    );
    match result {
        Err(AnalyzerError::Walk(msg)) => {
            assert!(
                msg.contains(PANIC_MARKER),
                "worker_threads={worker_threads}: expected the worker panic marker \
                 in the typed walk error, got: {msg}"
            );
        }
        Ok(ok) => panic!(
            "worker_threads={worker_threads}: expected a typed Walk error from a \
             panicking worker, but got Ok (cancelled={}, scanned_dirs={}) - a \
             truncated tree was returned",
            ok.cancelled, ok.progress.scanned_dirs
        ),
        Err(other) => {
            panic!("worker_threads={worker_threads}: expected AnalyzerError::Walk, got {other:?}")
        }
    }
}

#[test]
fn single_panicking_worker_surfaces_typed_walk_error() {
    assert_typed_walk_error(1);
}

#[test]
fn panicking_worker_with_peer_surfaces_typed_walk_error() {
    // With peers present, a panicking worker used to leave `work.active > 0`
    // and deadlock the join loop; the RAII work-slot guard must prevent it.
    assert_typed_walk_error(2);
}
