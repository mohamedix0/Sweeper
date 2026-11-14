//! Read-only process token elevation detection.
//!
//! Used by higher layers to decide whether elevation-only targets may be unlocked.
//! This module performs no filesystem writes and exposes no deletion primitives.

use crate::{Result, WinfsError};

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::mem;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{GetLastError, ERROR_INVALID_PARAMETER};
#[cfg(windows)]
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Returns `true` if the current process token is elevated.
///
/// This is a read-only query (no privilege escalation, no filesystem mutation).
pub fn is_process_elevated() -> Result<bool> {
    #[cfg(windows)]
    {
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle to the current process; it does not
        // require closing and is valid for the duration of the call.
        let process = unsafe { GetCurrentProcess() };
        let mut token_raw: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();
        // SAFETY: `process` is a valid process pseudo-handle and `token_raw` points to writable
        // storage for one HANDLE. On success, `token_raw` receives an open token handle that must be
        // closed exactly once, which is enforced by wrapping it in `SafeHandle`.
        let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token_raw) };
        if ok == 0 {
            // SAFETY: reads thread-local last-error state.
            let code = unsafe { GetLastError() };
            return Err(WinfsError::Win32 {
                context: "OpenProcessToken(GetCurrentProcess, TOKEN_QUERY)",
                code,
            });
        }
        let token = crate::SafeHandle::from_raw(token_raw);
        let mut elevation: TOKEN_ELEVATION = unsafe { mem::zeroed() };
        let mut returned_len: u32 = 0;
        let expected_len = mem::size_of::<TOKEN_ELEVATION>() as u32;
        // SAFETY: `token` is a valid open process token handle. `elevation` points to writable
        // storage of `TOKEN_ELEVATION` for `expected_len` bytes, and `returned_len` points to
        // writable storage for one u32. The buffer type/size match the `TokenElevation` class.
        let ok = unsafe {
            GetTokenInformation(
                token.raw(),
                TokenElevation,
                &mut elevation as *mut _ as *mut c_void,
                expected_len,
                &mut returned_len,
            )
        };
        if ok == 0 {
            // SAFETY: reads thread-local last-error state.
            let code = unsafe { GetLastError() };
            return Err(WinfsError::Win32 {
                context: "GetTokenInformation(TokenElevation)",
                code,
            });
        }
        interpret_token_elevation(elevation.TokenIsElevated, returned_len, expected_len)
    }
    #[cfg(not(windows))]
    {
        Err(WinfsError::Win32 {
            context: "is_process_elevated(unsupported platform)",
            code: 0,
        })
    }
}

/// Interprets a raw `TOKEN_ELEVATION.TokenIsElevated` value and validates the returned length.
///
/// Deterministic helper for unit tests (no syscalls).
pub(crate) fn interpret_token_elevation(
    token_is_elevated: u32,
    returned_len: u32,
    expected_len: u32,
) -> Result<bool> {
    if returned_len < expected_len {
        return Err(WinfsError::Win32 {
            context: "GetTokenInformation(TokenElevation) invalid return length",
            code: invalid_parameter_code(),
        });
    }
    Ok(token_is_elevated != 0)
}

#[cfg(windows)]
fn invalid_parameter_code() -> u32 {
    ERROR_INVALID_PARAMETER
}

#[cfg(not(windows))]
fn invalid_parameter_code() -> u32 {
    87 // ERROR_INVALID_PARAMETER
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::mem;
    use windows_sys::Win32::Security::TOKEN_ELEVATION;

    #[test]
    fn interpret_token_elevation_zero_is_not_elevated() {
        let expected = mem::size_of::<TOKEN_ELEVATION>() as u32;
        assert!(!interpret_token_elevation(0, expected, expected).unwrap());
    }

    #[test]
    fn interpret_token_elevation_nonzero_is_elevated() {
        let expected = mem::size_of::<TOKEN_ELEVATION>() as u32;
        assert!(interpret_token_elevation(1, expected, expected).unwrap());
        assert!(interpret_token_elevation(42, expected, expected).unwrap());
        // Defensive: larger returned length is acceptable.
        assert!(interpret_token_elevation(1, expected + 4, expected).unwrap());
    }

    #[test]
    fn interpret_token_elevation_rejects_short_length() {
        let expected = mem::size_of::<TOKEN_ELEVATION>() as u32;
        let err = interpret_token_elevation(1, expected.saturating_sub(1), expected).unwrap_err();
        match err {
            WinfsError::Win32 { context, code } => {
                assert!(context.contains("invalid return length"));
                assert_eq!(
                    code,
                    windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER
                );
            }
            other => panic!("expected Win32 error, got {other:?}"),
        }
    }

    #[test]
    fn is_process_elevated_returns_ok_without_panic() {
        let result = is_process_elevated();
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        let _ = result.unwrap();
    }
}
