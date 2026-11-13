//! Raw Windows NTFS operations (handle open, enumerate, reparse inspection).
//! Sole crate permitted to contain `unsafe`. No deletion primitives in Phase 0.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
