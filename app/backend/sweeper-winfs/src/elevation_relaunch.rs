//! Elevated relaunch via ShellExecuteExW("runas") and single-instance named mutex guard.
//!
//! These primitives perform no file deletion. Unsafe is confined to FFI calls with `// SAFETY:`.

use crate::{Result, WinfsError};

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::sync::mpsc::{self, Sender};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_CANCELLED, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
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

#[cfg(windows)]
#[derive(Debug)]
enum MutexCommand {
    Release(mpsc::Sender<Result<()>>),
    Reacquire {
        timeout_ms: u32,
        reply: mpsc::Sender<Result<()>>,
    },
    Shutdown,
}

/// RAII guard for the per-user single-instance named mutex.
///
/// All Win32 mutex operations (`CreateMutexW`, `WaitForSingleObject`, `ReleaseMutex`, `CloseHandle`)
/// execute on a dedicated owner thread to satisfy Windows mutex thread-affinity rules.
#[derive(Debug)]
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    tx: Option<Sender<MutexCommand>>,
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

    /// Acquire a named single-instance mutex (test/support seam for isolated parallel tests).
    pub fn acquire_named(mutex_name: &str, timeout_ms: u32) -> Result<Self> {
        #[cfg(windows)]
        {
            acquire_named_internal(mutex_name, timeout_ms)
        }
        #[cfg(not(windows))]
        {
            let _ = (mutex_name, timeout_ms);
            Err(unsupported())
        }
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
            let tx = self.tx.as_ref().ok_or(WinfsError::Win32 {
                context: "single-instance mutex owner thread unavailable",
                code: 0,
            })?;
            let (reply_tx, reply_rx) = mpsc::channel();
            tx.send(MutexCommand::Release(reply_tx))
                .map_err(|_| WinfsError::Win32 {
                    context: "single-instance mutex owner thread unavailable",
                    code: 0,
                })?;
            let res = reply_rx.recv().map_err(|_| WinfsError::Win32 {
                context: "single-instance mutex owner thread disconnected",
                code: 0,
            })?;
            if res.is_ok() {
                self.held = false;
            }
            res
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
            let tx = self.tx.as_ref().ok_or(WinfsError::Win32 {
                context: "single-instance mutex owner thread unavailable",
                code: 0,
            })?;
            let (reply_tx, reply_rx) = mpsc::channel();
            tx.send(MutexCommand::Reacquire {
                timeout_ms,
                reply: reply_tx,
            })
            .map_err(|_| WinfsError::Win32 {
                context: "single-instance mutex owner thread unavailable",
                code: 0,
            })?;
            let res = reply_rx.recv().map_err(|_| WinfsError::Win32 {
                context: "single-instance mutex owner thread disconnected",
                code: 0,
            })?;
            if res.is_ok() {
                self.held = true;
            }
            res
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
            if let Some(tx) = self.tx.take() {
                let _ = tx.send(MutexCommand::Shutdown);
            }
            self.held = false;
            // Handle close/release (if needed) performed by the dedicated owner thread.
        }
    }
}

/// Acquire or wait for the per-user single-instance mutex.
pub fn acquire_single_instance(timeout_ms: u32) -> Result<SingleInstanceGuard> {
    #[cfg(windows)]
    {
        acquire_named_internal(SINGLE_INSTANCE_MUTEX_NAME, timeout_ms)
    }
    #[cfg(not(windows))]
    {
        let _ = timeout_ms;
        Err(unsupported())
    }
}

#[cfg(windows)]
fn acquire_named_internal(mutex_name: &str, timeout_ms: u32) -> Result<SingleInstanceGuard> {
    let mutex_name = mutex_name.to_string();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
    let (cmd_tx, cmd_rx) = mpsc::channel::<MutexCommand>();

    let join = std::thread::Builder::new()
        .name("sweeper-single-instance-mutex".to_string())
        .spawn(move || run_mutex_owner_thread(&mutex_name, timeout_ms, ready_tx, cmd_rx))
        .map_err(|_| WinfsError::Win32 {
            context: "failed to spawn single-instance mutex owner thread",
            code: 0,
        })?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(SingleInstanceGuard {
            tx: Some(cmd_tx),
            held: true,
        }),
        Ok(Err(e)) => {
            let _ = join.join();
            Err(e)
        }
        Err(_) => {
            let _ = join.join();
            Err(WinfsError::Win32 {
                context: "single-instance mutex owner thread failed during startup",
                code: 0,
            })
        }
    }
}

#[cfg(windows)]
fn run_mutex_owner_thread(
    mutex_name: &str,
    timeout_ms: u32,
    ready_tx: mpsc::Sender<Result<()>>,
    cmd_rx: mpsc::Receiver<MutexCommand>,
) {
    let wide = to_wide_null(mutex_name);
    // SAFETY: `wide` is a NUL-terminated UTF-16 string valid for the call. Passing null security
    // attributes and bInitialOwner=FALSE is safe; the returned handle (if non-null) is owned by this
    // dedicated owner thread and closed exactly once before exit.
    let raw = unsafe { CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr()) };
    if raw.is_null() {
        let _ = ready_tx.send(Err(WinfsError::Win32 {
            context: "CreateMutexW(single-instance mutex)",
            code: unsafe { GetLastError() },
        }));
        return;
    }
    // SAFETY: capture last-error immediately after CreateMutexW (informational; not required here).
    let _create_err = unsafe { GetLastError() };

    let mut held = match wait_for_mutex_handle(raw, timeout_ms) {
        Ok(v) => v,
        Err(e) => {
            close_handle_raw(raw);
            let _ = ready_tx.send(Err(e));
            return;
        }
    };
    if ready_tx.send(Ok(())).is_err() {
        if held {
            let _ = unsafe { ReleaseMutex(raw) };
        }
        close_handle_raw(raw);
        return;
    }

    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            MutexCommand::Release(reply) => {
                if !held {
                    let _ = reply.send(Ok(()));
                    continue;
                }
                // SAFETY: `raw` is a valid mutex handle and this is the owning thread that acquired it.
                let ok = unsafe { ReleaseMutex(raw) };
                if ok == 0 {
                    let _ = reply.send(Err(WinfsError::Win32 {
                        context: "ReleaseMutex(single-instance handoff)",
                        code: unsafe { GetLastError() },
                    }));
                } else {
                    held = false;
                    let _ = reply.send(Ok(()));
                }
            }
            MutexCommand::Reacquire { timeout_ms, reply } => {
                if held {
                    let _ = reply.send(Ok(()));
                    continue;
                }
                match wait_for_mutex_handle(raw, timeout_ms) {
                    Ok(acquired) => {
                        held = acquired;
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            MutexCommand::Shutdown => break,
        }
    }

    if held {
        // SAFETY: best-effort release on shutdown from the owning thread only; ignore failure.
        let _ = unsafe { ReleaseMutex(raw) };
    }
    close_handle_raw(raw);
}

#[cfg(windows)]
fn wait_for_mutex_handle(handle: HANDLE, timeout_ms: u32) -> Result<bool> {
    // SAFETY: `handle` is a valid mutex handle owned by the calling (owner) thread context for wait.
    let rc = unsafe { WaitForSingleObject(handle, timeout_ms) };
    if rc == WAIT_OBJECT_0 || rc == WAIT_ABANDONED {
        return Ok(true);
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

#[cfg(windows)]
fn close_handle_raw(handle: HANDLE) {
    if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
        // SAFETY: `handle` is a valid open handle owned by the owner thread and closed exactly once.
        let _ = unsafe { CloseHandle(handle) };
    }
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_mutex_name(suffix: &str) -> String {
        static CTR: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        format!(r"Local\SweeperTest-{pid}-{nanos}-{n}-{suffix}")
    }

    #[test]
    fn single_instance_guard_release_reacquire_across_threads() {
        let name = unique_test_mutex_name("cross-thread");

        let guard = SingleInstanceGuard::acquire_named(&name, 1_000).expect("acquire on thread A");
        assert!(guard.is_held());

        let handle = std::thread::spawn(move || {
            let mut guard = guard;
            assert!(guard.is_held(), "guard should still be held on thread B");

            guard
                .release_for_handoff()
                .expect("release_for_handoff must succeed on thread B");

            // While released, another thread must be able to acquire the same named mutex.
            let contender = std::thread::spawn({
                let name = name.clone();
                move || SingleInstanceGuard::acquire_named(&name, 1_000)
            })
            .join()
            .expect("contender thread join");
            let contender = contender.expect("contender should acquire released mutex");
            drop(contender);

            guard
                .reacquire(1_000)
                .expect("reacquire must succeed on thread B");
            assert!(
                guard.is_held(),
                "guard should be held after reacquire on thread B"
            );
            guard
        });

        let guard = handle.join().expect("worker thread join");
        assert!(guard.is_held());
        drop(guard);
    }
}
