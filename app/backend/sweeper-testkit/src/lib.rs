//! Test harness: isolated sandbox with sentinel, canary harness, junction/symlink helpers.
//!
//! Phase 0: provides NTFS-safe test primitives without any deletion APIs.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::PathBuf;
use thiserror::Error;

mod canary;
mod cleanup;
mod ops;
mod owner_lock;
mod paths;
mod sandbox;

/// Errors returned by the testkit.
#[derive(Debug, Error)]
pub enum TestkitError {
    /// I/O error while creating or inspecting sandbox/canary state.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Walkdir traversal error.
    #[error("walk error: {0}")]
    Walk(String),
    /// Sandbox sentinel is missing.
    #[error("sandbox sentinel missing at {0}")]
    SentinelMissing(PathBuf),
    /// Canary hash verification failed.
    #[error("canary hash mismatch for {path}: expected {expected}, found {found}")]
    CanaryMismatch {
        /// Relative path within the canary tree.
        path: String,
        /// Expected SHA-256 hex digest.
        expected: String,
        /// Found SHA-256 hex digest.
        found: String,
    },
    /// Canary file missing compared to manifest.
    #[error("canary file missing: {0}")]
    CanaryMissing(String),
    /// Unexpected extra file in canary tree.
    #[error("canary unexpected file: {0}")]
    CanaryUnexpected(String),
}

/// Result type for testkit operations.
pub type Result<T> = std::result::Result<T, TestkitError>;

pub use canary::CanaryTree;
pub use cleanup::{
    cleanup_stale_canaries, cleanup_stale_sandboxes, CANARY_DIR_PREFIX, SANDBOX_DIR_PREFIX,
    SENTINEL_FILE_NAME,
};
#[cfg(not(windows))]
pub use ops::create_file_symlink;
pub use ops::{
    create_deep_tree, create_file, create_hard_link, create_junction, filetime_from_unix_secs,
};
#[cfg(windows)]
pub use ops::{
    create_file_symlink, filetime_now, read_ads, set_file_attributes, set_junction_on_open_handle,
    set_last_write_time, to_extended_length_path, write_ads,
};
pub use owner_lock::OWNER_LOCK_FILE_NAME;
pub use paths::{default_sandbox_base, default_sandbox_base_checked, is_safe_sandbox_base};
pub use sandbox::Sandbox;
