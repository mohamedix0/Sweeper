#![forbid(unsafe_code)]

pub mod analyzer;
pub mod journal;
pub mod paths;
pub mod policy;
pub mod remover;
pub mod scanner;
pub mod settings;
pub(crate) mod wildcard;

pub use scanner::{ScanError, ScanProgress, ScanProgressCallback};

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

/// Deterministic fuzz entrypoint for journal JSON-lines parsing.
/// Accepts arbitrary bytes and must never panic.
pub fn fuzz_journal_bytes(data: &[u8]) {
    let _ = journal::parse_records_bytes(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzz_journal_bytes_handles_malformed_binary_and_valid() {
        fuzz_journal_bytes(b"not json\n");
        fuzz_journal_bytes(&[0xff, 0x00, 0x7f]);
        let line = serde_json::to_string(&journal::intent_record("t", "C:\\a", 1, None)).unwrap();
        fuzz_journal_bytes(format!("{line}\n").as_bytes());
    }
}
