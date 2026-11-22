#![cfg(windows)]

//! Regression test for issue #134 (audit finding C4).
//!
//! A worker panic must be surfaced as a typed `AnalyzerError::Walk` instead of
//! being discarded by `let _ = h.join()` and silently returning a truncated
//! tree with `cancelled = false`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sweeper_core::analyzer::{analyze_path, AnalyzerError, AnalyzerOptions, AnalyzerProgress};

#[test]
fn panicking_worker_surfaces_typed_walk_error() {
    // Machine-neutral, real directory so root validation succeeds without
    // touching any real user/system path.
    let root = tempfile::tempdir().expect("create temp root");
    let root_str = root.path().to_string_lossy().to_string();

    // Panic exactly once: the first emit happens inside a worker thread; the
    // final emit (on the caller thread after joining) must not re-panic.
    let panicked = Arc::new(AtomicBool::new(false));
    let panicked_cb = Arc::clone(&panicked);
    let progress: Arc<dyn Fn(AnalyzerProgress) + Send + Sync> = Arc::new(move |_| {
        if !panicked_cb.swap(true, Ordering::SeqCst) {
            panic!("simulated analyzer worker panic (issue #134)");
        }
    });

    let result = analyze_path(
        Some(&root_str),
        Arc::new(AtomicBool::new(false)),
        Some(progress),
        AnalyzerOptions {
            worker_threads: 1,
            progress_interval: Duration::ZERO,
            ..AnalyzerOptions::default()
        },
    );

    assert!(
        panicked.load(Ordering::SeqCst),
        "the progress callback should have panicked inside the worker"
    );

    match result {
        Err(AnalyzerError::Walk(msg)) => {
            assert!(
                msg.contains("panic") || msg.contains("worker"),
                "expected worker panic detail in the typed walk error, got: {msg}"
            );
        }
        Ok(ok) => panic!(
            "expected a typed Walk error from a panicking worker, but got Ok \
             (cancelled={}, scanned_dirs={}) - a truncated tree was returned",
            ok.cancelled, ok.progress.scanned_dirs
        ),
        Err(other) => panic!("expected AnalyzerError::Walk, got {other:?}"),
    }
}
