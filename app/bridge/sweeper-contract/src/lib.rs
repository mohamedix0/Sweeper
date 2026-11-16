#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Contract DTOs, error types, command signatures, and TypeScript export for the Sweeper bridge.
//!
//! This crate is the single source of truth for IPC shapes shared by `sweeper-desktop` and
//! `frontend`. It has no dependency on `sweeper-winfs`, `sweeper-core`, or `sweeper-knowledge`.

mod bindings;
mod fuzz;
mod types;

#[cfg(test)]
mod tests;

pub use bindings::{export_ts_bindings, export_ts_bindings_to};
pub use fuzz::fuzz_contract_payload_bytes;
pub use types::*;
