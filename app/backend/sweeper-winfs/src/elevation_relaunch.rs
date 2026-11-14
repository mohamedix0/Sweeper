//! Elevated relaunch via ShellExecuteExW("runas") and single-instance named mutex guard.
//!
//! These primitives perform no file deletion. Unsafe is confined to FFI calls with `// SAFETY:`.

use crate::{Result, SafeHandle, WinfsError};

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_CANCELLED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
#[cfg(windows)]
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
#[cfg(windows)]
const SW_SHOWNORMAL: i32 = 1; // SW_SHOWNORMAL

#[cfg(windows)]
const SINGLE_INSTANCE_MUTEX_NAME: &str = r"Local\Sweeper-SingleInstance";

/// Result of attempting to relaunch the current executable elevated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaunchOutcome {
    /// A new elevated process launch was initiated.
    Launched,
    /// User cancelled the UAC consent prompt (or elevation was declined).
    CancelledByUser,
}

/// RAII guard for the per-user single-instance named mutex.
///
/// `held` indicates this process owns/holds the mutex lock (acquired via create+wait semantics).
#[derive(Debug)]
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    handle: SafeHandle,
    #[cfg(windows)]
    held: bool,
    #[cfg(not(windows))]
    _private: (),
}

impl SingleInstanceGuard {
    /// Acquire the single-instance mutex, waiting up to `timeout_ms` if another instance holds it.
    pub fn acquire(timeout_ms: u32) -> Result<Self> {
        acquire_single_instance(timeout_ms)
    }

    /// Try to acquire without blocking (0ms wait).
    pub fn try_acquire() -> Result<Self> {
        acquire_single_instance(0)
    }

    /// Returns true if this guard currently holds the mutex lock.
    pub fn is_held(&self) -> bool {
        #[cfg(windows)]
        {
            self.held
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Release the mutex lock while keeping the handle open (for handoff to a relaunched instance).
    /// No-op if not currently held.
    pub fn release_for_handoff(&mut self) -> Result<()> {
        #[cfg(windows)]
        {
            if !self.held {
                return Ok(());
            }
            // SAFETY: `self.handle` is a valid mutex handle owned by this guard. `ReleaseMutex` is
            // the correct API to release ownership acquired by this thread's successful wait/create.
            let ok = unsafe { ReleaseMutex(self.handle.raw()) };
            if ok == 0 {
                return Err(WinfsError::Win32 {
                    context: "ReleaseMutex(single-instance handoff)",
                    code: unsafe { GetLastError() },
                });
            }
            self.held = false;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = self;
            Err(unsupported())
        }
    }

    /// Re-acquire the mutex lock on an existing handle after a failed handoff (e.g. UAC cancel).
    pub fn reacquire(&mut self, timeout_ms: u32) -> Result<()> {
        #[cfg(windows)]
        {
            if self.held {
                return Ok(());
            }
            wait_for_mutex(&self.handle, timeout_ms)?;
            self.held = true;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (self, timeout_ms);
            Err(unsupported())
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if self.held {
                // SAFETY: best-effort release of a mutex we hold on drop. Ignoring failure is
                // acceptable here because process/thread exit semantics also release mutex ownership,
                // and we must not panic in Drop.
                let _ = unsafe { ReleaseMutex(self.handle.raw()) };
                self.held = false;
            }
            // Handle closed by SafeHandle drop.
        }
    }
}

/// Acquire or wait for the per-user single-instance mutex.
pub fn acquire_single_instance(timeout_ms: u32) -> Result<SingleInstanceGuard> {
    #[cfg(windows)]
    {
        let wide = to_wide_null(SINGLE_INSTANCE_MUTEX_NAME);
        // SAFETY: `wide` is a NUL-terminated UTF-16 string valid for the call. Passing null security
        // attributes and bInitialOwner=FALSE is safe; the returned handle (if non-null) is owned by us
        // and wrapped in SafeHandle for RAII CloseHandle.
        let raw = unsafe { CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr()) };
        if raw.is_null() {
            return Err(WinfsError::Win32 {
                context: "CreateMutexW(Local\\Sweeper-SingleInstance)",
                code: unsafe { GetLastError() },
            });
        }
        // SAFETY: capture last-error immediately after CreateMutexW to detect ERROR_ALREADY_EXISTS.
        let create_err = unsafe { GetLastError() };
        let handle = SafeHandle::from_raw(raw);
        let _already_exists = create_err == ERROR_ALREADY_EXISTS;
        wait_for_mutex(&handle, timeout_ms)?;
        Ok(SingleInstanceGuard { handle, held: true })
    }
    #[cfg(not(windows))]
    {
        let _ = timeout_ms;
        Err(unsupported())
    }
}

#[cfg(windows)]
fn wait_for_mutex(handle: &SafeHandle, timeout_ms: u32) -> Result<()> {
    // SAFETY: `handle` is a valid mutex handle. `WaitForSingleObject` blocks up to timeout_ms.
    let rc = unsafe { WaitForSingleObject(handle.raw(), timeout_ms) };
    if rc == WAIT_OBJECT_0 {
        return Ok(());
    }
    if rc == WAIT_TIMEOUT {
        return Err(WinfsError::Win32 {
            context: "WaitForSingleObject(single-instance mutex timeout)",
            code: WAIT_TIMEOUT,
        });
    }
    Err(WinfsError::Win32 {
        context: "WaitForSingleObject(single-instance mutex)",
        code: unsafe { GetLastError() },
    })
}

/// Relaunch the current executable with the `runas` verb to request elevation.
///
/// - Uses `std::env::current_exe()` and does not pass arbitrary command strings or deletion paths.
/// - Maps UAC cancellation to `RelaunchOutcome::CancelledByUser`.
pub fn relaunch_current_exe_elevated() -> Result<RelaunchOutcome> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| WinfsError::Win32 {
            context: "std::env::current_exe(relaunch_current_exe_elevated)",
            code: e.raw_os_error().unwrap_or(0) as u32,
        })?;
        let exe_w = to_wide_path(&exe);
        let verb_w = to_wide_null("runas");

        // SAFETY: zeroed SHELLEXECUTEINFOW is valid; we initialize required fields and pass pointers
        // to NUL-terminated UTF-16 buffers that remain valid for the duration of ShellExecuteExW.
        let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS;
        sei.lpVerb = verb_w.as_ptr();
        sei.lpFile = exe_w.as_ptr();
        sei.nShow = SW_SHOWNORMAL;

        // SAFETY: `sei` points to initialized SHELLEXECUTEINFOW with valid pointers and size.
        let ok = unsafe { ShellExecuteExW(&mut sei) };
        if ok == 0 {
            // SAFETY: read last-error immediately after failed ShellExecuteExW call.
            let code = unsafe { GetLastError() };
            if code == ERROR_CANCELLED {
                return Ok(RelaunchOutcome::CancelledByUser);
            }
            return Err(WinfsError::Win32 {
                context: "ShellExecuteExW(runas current_exe)",
                code,
            });
        }
        if !sei.hProcess.is_null() {
            // SAFETY: SEE_MASK_NOCLOSEPROCESS returned a process handle we own; close it to avoid leak.
            let _ = unsafe { CloseHandle(sei.hProcess) };
        }
        Ok(RelaunchOutcome::Launched)
    }
    #[cfg(not(windows))]
    {
        Err(unsupported())
    }
}

#[cfg(windows)]
fn to_wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn to_wide_path(p: &std::path::Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(not(windows))]
fn unsupported() -> WinfsError {
    WinfsError::Win32 {
        context: "unsupported platform",
        code: 0,
    }
}
