#![forbid(unsafe_code)]

pub mod analyzer;
pub mod journal;
pub mod paths;
pub mod policy;
pub mod remover;
pub mod scanner;
pub mod settings;

/// Re-exported knowledge API/types for bridge/session layers (avoids direct sweeper-knowledge dependency).
pub mod knowledge {
    pub use sweeper_knowledge::*;
}
pub use sweeper_knowledge::{Risk, Tier};

/// Elevation context for scan/clean eligibility decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanElevationContext {
    /// True when the current process is running elevated (administrator).
    pub process_elevated: bool,
}

/// Process elevation, elevated relaunch, and single-instance primitives (safe wrappers over `sweeper-winfs`).
pub mod elevation {
    pub use sweeper_winfs::{
        acquire_single_instance, is_process_elevated, relaunch_current_exe_elevated,
        RelaunchOutcome, SingleInstanceGuard,
    };
}

/// Deterministic fuzz entrypoint for path/policy validation.
/// Accepts arbitrary bytes (lossy UTF-8) and must never panic.
pub fn fuzz_path_policy_bytes(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    let _ = paths::CanonicalPath::parse_lexical(&s);
    let _ = policy::validate_custom_path_lexical(&s);
}
