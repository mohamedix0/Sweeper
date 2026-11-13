#![forbid(unsafe_code)]

pub mod paths;
pub mod policy;

/// Deterministic fuzz entrypoint for path/policy validation.
/// Accepts arbitrary bytes (lossy UTF-8) and must never panic.
pub fn fuzz_path_policy_bytes(data: &[u8]) {
    let s = String::from_utf8_lossy(data);
    let _ = paths::CanonicalPath::parse_lexical(&s);
    let _ = policy::validate_custom_path_lexical(&s);
}
