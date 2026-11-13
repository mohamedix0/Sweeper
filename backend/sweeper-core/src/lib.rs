//! Core logic: validation, Scan, Analyzer, Remover, Journal, Settings.
//! `#![forbid(unsafe_code)]`. No deletion primitives or execute_clean in Phase 0.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

fn _gate_probe() {
    let _ = std::fs::remove_file("C:\\should-not-exist");
}
