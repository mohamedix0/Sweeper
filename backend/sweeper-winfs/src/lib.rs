//! Raw Windows NTFS operations (read-only handle inspection and handle-based deletion primitives).
//! Sole crate permitted to contain `unsafe`. All deletion primitives are invoked only by
//! `sweeper-core::remover` (enforced by clippy/gate-a checks).

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use thiserror::Error;
use windows_sys::core::GUID;
#[cfg(any(test, feature = "test-support"))]
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ENVVAR_NOT_FOUND, ERROR_FILE_NOT_FOUND,
    ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, ERROR_NO_MORE_FILES,
    ERROR_PATH_NOT_FOUND, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DeleteFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    SetFileInformationByHandle, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FIND_FIRST_EX_LARGE_FETCH, OPEN_EXISTING, WIN32_FIND_DATAW,
};
#[cfg(any(test, feature = "test-support"))]
use windows_sys::Win32::Storage::FileSystem::{
    RemoveDirectoryW, SetFileTime, FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Environment::GetEnvironmentVariableW;
use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
#[cfg(any(test, feature = "test-support"))]
use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};

/// Reparse tag: symbolic link.
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
/// Reparse tag: mount point / junction.
pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
/// Cloud placeholder tag (OneDrive / cldflt).
pub const IO_REPARSE_TAG_CLOUD: u32 = 0x9000_001A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_1: u32 = 0x9000_101A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_2: u32 = 0x9000_201A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_3: u32 = 0x9000_301A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_4: u32 = 0x9000_401A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_5: u32 = 0x9000_501A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_6: u32 = 0x9000_601A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_7: u32 = 0x9000_701A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_8: u32 = 0x9000_801A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_9: u32 = 0x9000_901A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_A: u32 = 0x9000_A01A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_B: u32 = 0x9000_B01A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_C: u32 = 0x9000_C01A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_D: u32 = 0x9000_D01A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_E: u32 = 0x9000_E01A;
/// Cloud placeholder tag variant.
pub const IO_REPARSE_TAG_CLOUD_F: u32 = 0x9000_F01A;

// File disposition constants (not all exposed by windows-sys 0.59 in every feature set).
const FILE_DISPOSITION_INFO_CLASS: i32 = 4; // FileDispositionInfo
const FILE_DISPOSITION_INFO_EX_CLASS: i32 = 21; // FileDispositionInfoEx
const FILE_BASIC_INFO_CLASS: i32 = 0; // FileBasicInfo
const FILE_DISPOSITION_FLAG_DELETE: u32 = 0x0000_0001;
const FILE_DISPOSITION_FLAG_POSIX_SEMANTICS: u32 = 0x0000_0002;
const FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE: u32 = 0x0000_0010;
const DELETE_ACCESS: u32 = 0x0001_0000;
const FILE_READ_ATTRIBUTES_ACCESS: u32 = 0x0000_0080;
const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;

/// Errors returned by `sweeper-winfs` operations.
#[derive(Debug, Error)]
pub enum WinfsError {
    /// Win32 API failure with context and last OS error code.
    #[error("{context} failed (os error {code})")]
    Win32 {
        /// Operation description.
        context: &'static str,
        /// Raw Win32 error code from `GetLastError`.
        code: u32,
    },
    /// Requested environment variable was not found.
    #[error("environment variable not found: {0}")]
    EnvVarNotFound(String),
    /// UTF-16 / path conversion issue.
    #[error("invalid utf-16 or path encoding: {0}")]
    InvalidEncoding(String),
}

/// Result type for winfs operations.
pub type Result<T> = std::result::Result<T, WinfsError>;

/// RAII wrapper around a Win32 `HANDLE` closed via `CloseHandle` on drop.
#[derive(Debug)]
pub struct SafeHandle(HANDLE);

impl SafeHandle {
    /// Returns the raw handle for FFI calls.
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for SafeHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: `self.0` is a valid open handle owned by this `SafeHandle` (or null/invalid
            // sentinel already guarded). `CloseHandle` is the correct API to release it exactly once
            // during drop; no other code closes or aliases ownership after construction.
            let ok = unsafe { CloseHandle(self.0) };
            debug_assert!(ok != 0, "CloseHandle failed in SafeHandle::drop");
        }
    }
}

/// Classification of a reparse point / directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReparseTagClass {
    /// Not a reparse point.
    NotReparse,
    /// A link point (symlink or mount point/junction). Never followed.
    LinkPoint,
    /// Cloud placeholder (OneDrive/cldflt). Never hydrated, always treated as skipped by policy.
    CloudPlaceholder,
    /// Other reparse tag, preserved for reporting/debugging.
    OtherReparse(u32),
}

/// File identity from `GetFileInformationByHandle` (volume serial + file index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileIdentity {
    /// Volume serial number (`dwVolumeSerialNumber`).
    pub volume_serial: u32,
    /// 64-bit file index composed from high/low parts.
    pub file_index: u64,
}

/// Read-only directory entry metadata.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Entry name (final path component).
    pub name: OsString,
    /// Full path as observed during enumeration.
    pub path: PathBuf,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Logical file size in bytes (0 for directories / as reported).
    pub size: u64,
    /// Raw file attributes.
    pub attributes: u32,
    /// Last write time as raw FILETIME `(dwHighDateTime, dwLowDateTime)`.
    pub last_write_time: (u32, u32),
    /// Creation time as raw FILETIME `(dwHighDateTime, dwLowDateTime)`.
    pub creation_time: (u32, u32),
    /// Last access time as raw FILETIME `(dwHighDateTime, dwLowDateTime)`.
    pub last_access_time: (u32, u32),
    /// Reparse classification (from find data / tag).
    pub reparse: ReparseTagClass,
    /// Raw reparse tag when applicable.
    pub reparse_tag: u32,
}

/// Snapshot of handle-observed metadata for moment-of-deletion re-validation.
#[derive(Debug, Clone)]
pub struct HandleInspection {
    /// Canonical final path resolved from the opened handle.
    pub final_path: String,
    /// File identity (volume serial + file index).
    pub identity: FileIdentity,
    /// Raw file attributes.
    pub attributes: u32,
    /// Logical file size in bytes (0 for directories).
    pub size: u64,
    /// Reparse classification determined from handle attributes/tag.
    pub reparse: ReparseTagClass,
    /// Raw reparse tag if applicable (0 if none).
    pub reparse_tag: u32,
    /// Last write time FILETIME `(high, low)` as observed on the handle.
    pub last_write_time: (u32, u32),
}

/// Open a file or directory handle for read-only inspection without following reparse points.
///
/// Uses `FILE_FLAG_OPEN_REPARSE_POINT` and `FILE_FLAG_BACKUP_SEMANTICS` (for directories)
/// with shared read/write/delete access and `OPEN_EXISTING`.
pub fn open_for_inspection(path: &Path) -> Result<SafeHandle> {
    let wide = to_wide(path);
    let flags = FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS;
    // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that lives for the duration of the call.
    // We request no data access rights (metadata APIs used here do not require them) and open with
    // broad share flags for read-only inspection. `FILE_FLAG_OPEN_REPARSE_POINT` ensures link
    // points are not followed. `OPEN_EXISTING` opens only existing filesystem objects.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(last_error("CreateFileW(open_for_inspection)"));
    }
    Ok(SafeHandle(handle))
}

/// Open a file or directory handle for handle-based deletion without following reparse points.
///
/// Uses `FILE_FLAG_OPEN_REPARSE_POINT` and `FILE_FLAG_BACKUP_SEMANTICS`, requests
/// `DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE`, and share flags appropriate for safe deletion.
pub fn open_for_delete(path: &Path) -> Result<SafeHandle> {
    let wide = to_wide(path);
    let flags = FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS;
    // Include FILE_WRITE_ATTRIBUTES to allow clearing read-only via handle-based basic info update.
    const FILE_WRITE_ATTRIBUTES_ACCESS: u32 = 0x0000_0100;
    let access = DELETE_ACCESS
        | FILE_READ_ATTRIBUTES_ACCESS
        | FILE_WRITE_ATTRIBUTES_ACCESS
        | SYNCHRONIZE_ACCESS;
    // SAFETY: `wide` is a NUL-terminated UTF-16 buffer valid for the call. The handle is opened
    // with `OPEN_EXISTING` and `FILE_FLAG_OPEN_REPARSE_POINT` so reparse points (including link
    // points) are not traversed. `DELETE` access is required for disposition-based deletion; share
    // flags allow concurrent readers/writers where possible while still failing if another opener
    // denies delete sharing (surfaced as a lock/skipped condition by callers).
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            flags,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(last_error("CreateFileW(open_for_delete)"));
    }
    Ok(SafeHandle(handle))
}

/// Resolve canonical path for an opened handle via `GetFinalPathNameByHandleW`.
pub fn get_final_path(handle: &SafeHandle) -> Result<String> {
    // VOLUME_NAME_DOS = 0x0, FILE_NAME_NORMALIZED = 0x0
    const VOLUME_NAME_DOS: u32 = 0;
    const FILE_NAME_NORMALIZED: u32 = 0;
    let flags = VOLUME_NAME_DOS | FILE_NAME_NORMALIZED;
    // SAFETY: querying required length with null buffer and size 0 is valid for a live handle.
    let needed = unsafe { GetFinalPathNameByHandleW(handle.raw(), std::ptr::null_mut(), 0, flags) };
    if needed == 0 {
        return Err(last_error("GetFinalPathNameByHandleW(len)"));
    }
    let mut buf = vec![0u16; needed as usize];
    // SAFETY: `buf` is writable for `needed` UTF-16 code units and `handle` is valid/open.
    let written =
        unsafe { GetFinalPathNameByHandleW(handle.raw(), buf.as_mut_ptr(), needed, flags) };
    if written == 0 {
        return Err(last_error("GetFinalPathNameByHandleW"));
    }
    let len = written as usize;
    let slice = &buf[..len];
    String::from_utf16(slice).map_err(|e| WinfsError::InvalidEncoding(e.to_string()))
}

/// Get file identity (volume serial + file index) via `GetFileInformationByHandle`.
pub fn get_file_identity(handle: &SafeHandle) -> Result<FileIdentity> {
    // SAFETY: `info` is zero-initialized writable storage of the exact structure expected by the API.
    let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
        unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is valid and `info` points to writable memory of the correct type/size.
    let ok = unsafe { GetFileInformationByHandle(handle.raw(), &mut info) };
    if ok == 0 {
        return Err(last_error("GetFileInformationByHandle"));
    }
    let file_index = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
    Ok(FileIdentity {
        volume_serial: info.dwVolumeSerialNumber,
        file_index,
    })
}

/// Collect handle-observed inspection data for moment-of-deletion re-validation.
pub fn inspect_handle(handle: &SafeHandle) -> Result<HandleInspection> {
    let final_path = get_final_path(handle)?;
    // SAFETY: zero-initialized BY_HANDLE_FILE_INFORMATION is valid writable storage for the call.
    let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
        unsafe { std::mem::zeroed() };
    // SAFETY: handle is valid and `info` is a correctly sized writable output buffer.
    let ok = unsafe { GetFileInformationByHandle(handle.raw(), &mut info) };
    if ok == 0 {
        return Err(last_error("GetFileInformationByHandle(inspect)"));
    }
    let identity = FileIdentity {
        volume_serial: info.dwVolumeSerialNumber,
        file_index: ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64),
    };
    let attributes = info.dwFileAttributes;
    let size = ((info.nFileSizeHigh as u64) << 32) | (info.nFileSizeLow as u64);

    // Deterministic test override (path-keyed) takes precedence when present.
    if let Some(ov) = test_reparse_override_for_path(&final_path) {
        let reparse_tag = match ov {
            ReparseTagClass::OtherReparse(t) => t,
            ReparseTagClass::CloudPlaceholder => IO_REPARSE_TAG_CLOUD,
            ReparseTagClass::LinkPoint => IO_REPARSE_TAG_SYMLINK,
            ReparseTagClass::NotReparse => 0,
        };
        return Ok(HandleInspection {
            final_path,
            identity,
            attributes,
            size,
            reparse: ov,
            reparse_tag,
            last_write_time: (
                info.ftLastWriteTime.dwHighDateTime,
                info.ftLastWriteTime.dwLowDateTime,
            ),
        });
    }

    let mut reparse = if (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        inspect_reparse_point(handle)?
    } else {
        ReparseTagClass::NotReparse
    };
    let mut reparse_tag = match reparse {
        ReparseTagClass::NotReparse => 0,
        ReparseTagClass::LinkPoint => 0,
        ReparseTagClass::CloudPlaceholder => 0,
        ReparseTagClass::OtherReparse(tag) => tag,
    };
    if (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 && reparse_tag == 0 {
        if let Some(tag) = query_reparse_tag(handle)? {
            reparse_tag = tag;
            reparse = classify_reparse_tag(tag);
        } else {
            reparse = ReparseTagClass::OtherReparse(0);
            reparse_tag = 0;
        }
    }
    Ok(HandleInspection {
        final_path,
        identity,
        attributes,
        size,
        reparse,
        reparse_tag,
        last_write_time: (
            info.ftLastWriteTime.dwHighDateTime,
            info.ftLastWriteTime.dwLowDateTime,
        ),
    })
}

fn query_reparse_tag(handle: &SafeHandle) -> Result<Option<u32>> {
    let mut buf = vec![0u8; 16 * 1024];
    let mut bytes_returned: u32 = 0;
    // SAFETY: handle is valid; input buffer is null/0 for FSCTL_GET_REPARSE_POINT and output buffer
    // is writable for `buf.len()` bytes with a valid bytes-returned pointer.
    let ok = unsafe {
        DeviceIoControl(
            handle.raw(),
            FSCTL_GET_REPARSE_POINT,
            std::ptr::null(),
            0,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        const ERROR_NOT_A_REPARSE_POINT: u32 = 4390;
        if err == ERROR_NOT_A_REPARSE_POINT {
            return Ok(None);
        }
        return Err(WinfsError::Win32 {
            context: "DeviceIoControl(FSCTL_GET_REPARSE_POINT)",
            code: err,
        });
    }
    if (bytes_returned as usize) < std::mem::size_of::<u32>() {
        return Ok(Some(0));
    }
    // SAFETY: at least 4 bytes returned; first DWORD of reparse data buffer is the reparse tag.
    let tag = unsafe { *(buf.as_ptr() as *const u32) };
    Ok(Some(tag))
}

/// Clear the read-only attribute on an opened handle (best-effort support for deletion in valid targets).
pub fn clear_readonly_attribute(handle: &SafeHandle) -> Result<()> {
    let attrs = handle_file_attributes(handle)?;
    if (attrs & FILE_ATTRIBUTE_READONLY) == 0 {
        return Ok(());
    }
    let mut new_attrs = attrs & !FILE_ATTRIBUTE_READONLY;
    if new_attrs == 0 {
        // FILE_BASIC_INFO.FileAttributes == 0 means "no change"; use FILE_ATTRIBUTE_NORMAL for files.
        new_attrs = 0x80; // FILE_ATTRIBUTE_NORMAL
    }
    // Ensure directory attribute is preserved if set.
    if (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0 {
        new_attrs |= FILE_ATTRIBUTE_DIRECTORY;
        new_attrs &= !0x80;
        if new_attrs == 0 {
            new_attrs = FILE_ATTRIBUTE_DIRECTORY;
        }
    }
    // SAFETY: zero-initialized FILE_BASIC_INFO is valid; zero time fields mean "do not change".
    let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
    basic.FileAttributes = new_attrs;
    // SAFETY: handle is valid and `basic` points to initialized FILE_BASIC_INFO of correct size.
    let set_ok = unsafe {
        SetFileInformationByHandle(
            handle.raw(),
            FILE_BASIC_INFO_CLASS,
            &basic as *const _ as *const _,
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if set_ok == 0 {
        return Err(last_error(
            "SetFileInformationByHandle(FileBasicInfo clear readonly)",
        ));
    }
    Ok(())
}

/// Mark an opened handle for deletion using `FileDispositionInfoEx` with POSIX semantics.
///
/// On `ERROR_INVALID_FUNCTION` / `ERROR_NOT_SUPPORTED`, falls back to `FileDispositionInfo`.
/// Never falls back to path-based deletion.
pub fn set_file_disposition_delete(handle: &SafeHandle) -> Result<()> {
    #[repr(C)]
    struct FileDispositionInfoEx {
        flags: u32,
    }
    #[repr(C)]
    struct FileDispositionInfo {
        delete_file: u8,
    }

    let info_ex = FileDispositionInfoEx {
        flags: FILE_DISPOSITION_FLAG_DELETE
            | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
            | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    };
    // SAFETY: handle is valid and points to initialized FileDispositionInfoEx for correct class/size.
    let ok = unsafe {
        SetFileInformationByHandle(
            handle.raw(),
            FILE_DISPOSITION_INFO_EX_CLASS,
            &info_ex as *const _ as *const _,
            std::mem::size_of::<FileDispositionInfoEx>() as u32,
        )
    };
    if ok != 0 {
        return Ok(());
    }
    let err = unsafe { GetLastError() };
    if err == ERROR_INVALID_FUNCTION || err == ERROR_NOT_SUPPORTED {
        let info = FileDispositionInfo { delete_file: 1 };
        // SAFETY: handle is valid; fallback uses documented FileDispositionInfo class/size only.
        let ok2 = unsafe {
            SetFileInformationByHandle(
                handle.raw(),
                FILE_DISPOSITION_INFO_CLASS,
                &info as *const _ as *const _,
                std::mem::size_of::<FileDispositionInfo>() as u32,
            )
        };
        if ok2 != 0 {
            return Ok(());
        }
        return Err(WinfsError::Win32 {
            context: "SetFileInformationByHandle(FileDispositionInfo fallback)",
            code: unsafe { GetLastError() },
        });
    }
    Err(WinfsError::Win32 {
        context: "SetFileInformationByHandle(FileDispositionInfoEx)",
        code: err,
    })
}

// File/directory deletion is performed by `sweeper-core::remover` via `open_for_delete` + disposition.

/// Returns true if a Win32 error code indicates a sharing/lock condition for delete attempts.
pub fn is_sharing_violation(code: u32) -> bool {
    // ERROR_SHARING_VIOLATION = 32
    code == 32 || code == ERROR_ACCESS_DENIED
}

/// Get raw attributes for an opened handle.
pub fn handle_file_attributes(handle: &SafeHandle) -> Result<u32> {
    // SAFETY: zeroed output struct is valid for GetFileInformationByHandle.
    let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
        unsafe { std::mem::zeroed() };
    // SAFETY: handle is valid and output buffer is correctly typed/sized.
    let ok = unsafe { GetFileInformationByHandle(handle.raw(), &mut info) };
    if ok == 0 {
        return Err(last_error("GetFileInformationByHandle(attributes)"));
    }
    Ok(info.dwFileAttributes)
}

/// Read-only handle-aware directory enumeration.
///
/// Uses `FindFirstFileExW` with `FIND_FIRST_EX_LARGE_FETCH`. Never follows `Link Point`s:
/// entries are returned as-is with reparse classification so callers can avoid traversal.
pub fn read_dir_entries(dir: &Path) -> Result<Vec<DirEntry>> {
    let pattern = dir.join("*");
    let wide = to_wide(&pattern);
    // SAFETY: zeroed WIN32_FIND_DATAW is valid writable output for find APIs.
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    // SAFETY: `wide` is NUL-terminated and valid for the call; `data` is writable. Enumeration is
    // read-only and lists entries in `dir` without traversing reparse targets.
    let handle = unsafe {
        FindFirstFileExW(
            wide.as_ptr(),
            FindExInfoBasic,
            &mut data as *mut _ as *mut _,
            FindExSearchNameMatch,
            std::ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        let err = unsafe { GetLastError() };
        if err == ERROR_FILE_NOT_FOUND || err == ERROR_PATH_NOT_FOUND {
            return Ok(Vec::new());
        }
        return Err(WinfsError::Win32 {
            context: "FindFirstFileExW",
            code: err,
        });
    }
    let mut entries = Vec::new();
    loop {
        let name = os_string_from_wide_buf(&data.cFileName);
        if name != OsStr::new(".") && name != OsStr::new("..") {
            entries.push(dir_entry_from_find_data(dir, &data, name));
        }
        // SAFETY: find handle and data buffer are valid for advancing enumeration.
        let ok = unsafe { FindNextFileW(handle, &mut data) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            // SAFETY: close the valid find handle exactly once before returning/breaking.
            unsafe { FindClose(handle) };
            if err == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(WinfsError::Win32 {
                context: "FindNextFileW",
                code: err,
            });
        }
    }
    Ok(entries)
}

/// Classify a reparse tag value.
pub fn classify_reparse_tag(tag: u32) -> ReparseTagClass {
    match tag {
        0 => ReparseTagClass::NotReparse,
        IO_REPARSE_TAG_SYMLINK | IO_REPARSE_TAG_MOUNT_POINT => ReparseTagClass::LinkPoint,
        IO_REPARSE_TAG_CLOUD
        | IO_REPARSE_TAG_CLOUD_1
        | IO_REPARSE_TAG_CLOUD_2
        | IO_REPARSE_TAG_CLOUD_3
        | IO_REPARSE_TAG_CLOUD_4
        | IO_REPARSE_TAG_CLOUD_5
        | IO_REPARSE_TAG_CLOUD_6
        | IO_REPARSE_TAG_CLOUD_7
        | IO_REPARSE_TAG_CLOUD_8
        | IO_REPARSE_TAG_CLOUD_9
        | IO_REPARSE_TAG_CLOUD_A
        | IO_REPARSE_TAG_CLOUD_B
        | IO_REPARSE_TAG_CLOUD_C
        | IO_REPARSE_TAG_CLOUD_D
        | IO_REPARSE_TAG_CLOUD_E
        | IO_REPARSE_TAG_CLOUD_F => ReparseTagClass::CloudPlaceholder,
        // Conservative heuristic for cldflt/cloud placeholder families (0x9000_x01A-style).
        other if (other & 0xF000_FFFF) == 0x9000_001A => ReparseTagClass::CloudPlaceholder,
        other => ReparseTagClass::OtherReparse(other),
    }
}

/// Inspect reparse tag for an opened handle via `FSCTL_GET_REPARSE_POINT`.
///
/// Returns `ReparseTagClass::NotReparse` if the handle/path is not a reparse point.
pub fn inspect_reparse_point(handle: &SafeHandle) -> Result<ReparseTagClass> {
    if let Some(ov) = test_reparse_override_for_handle(handle) {
        return Ok(ov);
    }
    let mut buf = vec![0u8; 16 * 1024];
    let mut bytes_returned: u32 = 0;
    // SAFETY: handle is valid; output buffer and bytes_returned are writable for the call.
    let ok = unsafe {
        DeviceIoControl(
            handle.raw(),
            FSCTL_GET_REPARSE_POINT,
            std::ptr::null(),
            0,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        const ERROR_NOT_A_REPARSE_POINT: u32 = 4390;
        if err == ERROR_NOT_A_REPARSE_POINT {
            return Ok(ReparseTagClass::NotReparse);
        }
        return Err(WinfsError::Win32 {
            context: "DeviceIoControl(FSCTL_GET_REPARSE_POINT)",
            code: err,
        });
    }
    if (bytes_returned as usize) < std::mem::size_of::<u32>() {
        return Ok(ReparseTagClass::OtherReparse(0));
    }
    // SAFETY: buffer contains at least 4 bytes; first field of reparse data buffer is the tag.
    let tag = unsafe { *(buf.as_ptr() as *const u32) };
    Ok(classify_reparse_tag(tag))
}

/// Known Folder identifiers required for Sweeper allowlist resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownFolderId {
    /// `FOLDERID_LocalAppData`
    LocalAppData,
    /// `FOLDERID_RoamingAppData`
    RoamingAppData,
    /// `FOLDERID_Profile` (user profile root)
    UserProfile,
    /// `FOLDERID_Windows`
    Windows,
    /// `FOLDERID_ProgramData`
    ProgramData,
}

impl KnownFolderId {
    fn guid(self) -> GUID {
        match self {
            Self::LocalAppData => GUID {
                data1: 0xF1B3_2785,
                data2: 0x6FBA,
                data3: 0x4FCF,
                data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
            },
            Self::RoamingAppData => GUID {
                data1: 0x3EB6_85DB,
                data2: 0x65F9,
                data3: 0x4CF6,
                data4: [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
            },
            Self::UserProfile => GUID {
                data1: 0x5E6C_858F,
                data2: 0x0E22,
                data3: 0x4760,
                data4: [0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73],
            },
            Self::Windows => GUID {
                data1: 0xF38B_F404,
                data2: 0x1D43,
                data3: 0x42F2,
                data4: [0x93, 0x05, 0x67, 0xDE, 0x0B, 0x28, 0xFC, 0x23],
            },
            Self::ProgramData => GUID {
                data1: 0x62AB_5D82,
                data2: 0xFDC1,
                data3: 0x4DC3,
                data4: [0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97],
            },
        }
    }
}

/// Resolve a known folder to its filesystem path using `SHGetKnownFolderPath`.
pub fn resolve_known_folder(id: KnownFolderId) -> Result<PathBuf> {
    let guid = id.guid();
    known_folder_path(&guid)
}

/// Resolve a Known Folder path from a GUID using `SHGetKnownFolderPath`.
pub fn known_folder_path(folder_id: &GUID) -> Result<PathBuf> {
    let mut path_ptr: *mut u16 = std::ptr::null_mut();
    // SAFETY: `folder_id` is a valid GUID pointer for the call. On success `path_ptr` receives a
    // CoTaskMem-allocated NUL-terminated UTF-16 string that must be freed with CoTaskMemFree.
    let hr = unsafe {
        SHGetKnownFolderPath(
            folder_id,
            KF_FLAG_DEFAULT as u32,
            std::ptr::null_mut(),
            &mut path_ptr,
        )
    };
    if hr < 0 {
        return Err(WinfsError::Win32 {
            context: "SHGetKnownFolderPath",
            code: hr as u32,
        });
    }
    if path_ptr.is_null() {
        return Err(WinfsError::Win32 {
            context: "SHGetKnownFolderPath(null path)",
            code: 0,
        });
    }
    // SAFETY: on success `path_ptr` is a valid NUL-terminated UTF-16 string allocated by COM.
    let path = unsafe {
        let mut len = 0usize;
        while *path_ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(path_ptr, len);
        let os = OsString::from_wide(slice);
        PathBuf::from(os)
    };
    // SAFETY: free the CoTaskMem-allocated buffer exactly once.
    unsafe { CoTaskMemFree(path_ptr as *const _) };
    Ok(path)
}

/// Read-only environment variable helper using `GetEnvironmentVariableW`.
pub fn env_var(name: &str) -> Option<String> {
    let wide_name = to_wide_os(OsStr::new(name));
    // SAFETY: querying required size with null buffer/0 is valid; `wide_name` is NUL-terminated.
    let needed = unsafe { GetEnvironmentVariableW(wide_name.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        let err = unsafe { GetLastError() };
        if err == ERROR_ENVVAR_NOT_FOUND {
            return None;
        }
        return None;
    }
    let mut buf = vec![0u16; needed as usize];
    // SAFETY: buffer is writable for `needed` elements for the duration of the call.
    let written = unsafe { GetEnvironmentVariableW(wide_name.as_ptr(), buf.as_mut_ptr(), needed) };
    if written == 0 {
        return None;
    }
    let slice = &buf[..written as usize];
    String::from_utf16(slice).ok()
}

// (intentionally no extra FFI; keep unsafe surface minimal and use available windows-sys APIs)

fn dir_entry_from_find_data(dir: &Path, data: &WIN32_FIND_DATAW, name: OsString) -> DirEntry {
    let attrs = data.dwFileAttributes;
    let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY) != 0;
    let reparse_tag = data.dwReserved0;
    let reparse = if (attrs & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
        classify_reparse_tag(reparse_tag)
    } else {
        ReparseTagClass::NotReparse
    };
    let size = ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64);
    DirEntry {
        path: dir.join(&name),
        name,
        is_dir,
        size,
        attributes: attrs,
        last_write_time: (
            data.ftLastWriteTime.dwHighDateTime,
            data.ftLastWriteTime.dwLowDateTime,
        ),
        creation_time: (
            data.ftCreationTime.dwHighDateTime,
            data.ftCreationTime.dwLowDateTime,
        ),
        last_access_time: (
            data.ftLastAccessTime.dwHighDateTime,
            data.ftLastAccessTime.dwLowDateTime,
        ),
        reparse,
        reparse_tag: if (attrs & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            reparse_tag
        } else {
            0
        },
    }
}

fn to_wide(path: &Path) -> Vec<u16> {
    #[cfg(windows)]
    {
        let s = path.as_os_str();
        let text = s.to_string_lossy();
        if text.len() >= 260
            && !text.starts_with(r"\\?\")
            && !text.starts_with(r"\\.\")
            && text.as_bytes().get(1).copied() == Some(b':')
        {
            let prefixed = format!(r"\\?\{}", text);
            return to_wide_os(OsStr::new(&prefixed));
        }
    }
    to_wide_os(path.as_os_str())
}

fn to_wide_os(s: &OsStr) -> Vec<u16> {
    let mut wide: Vec<u16> = s.encode_wide().collect();
    wide.push(0);
    wide
}

fn os_string_from_wide_buf(buf: &[u16]) -> OsString {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..len])
}

fn last_error(context: &'static str) -> WinfsError {
    // SAFETY: reads thread-local last-error state with no preconditions.
    let code = unsafe { GetLastError() };
    WinfsError::Win32 { context, code }
}

/* test hook state helpers defined below */

fn test_reparse_override_map() -> &'static Mutex<HashMap<String, ReparseTagClass>> {
    static MAP: OnceLock<Mutex<HashMap<String, ReparseTagClass>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_test_path_key(path: &str) -> String {
    path.trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_ascii_uppercase()
}

fn test_reparse_override_for_path(path: &str) -> Option<ReparseTagClass> {
    let key = normalize_test_path_key(path);
    let map = test_reparse_override_map().lock().ok()?;
    if let Some(v) = map.get(&key).copied() {
        return Some(v);
    }
    // Also try lexical-normalized form if path is parseable.
    if let Ok(p) =
        std::path::PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(path)).canonicalize()
    {
        let k2 = normalize_test_path_key(&p.to_string_lossy());
        return map.get(&k2).copied();
    }
    None
}

fn test_reparse_override_for_handle(handle: &SafeHandle) -> Option<ReparseTagClass> {
    let path = get_final_path(handle).ok()?;
    test_reparse_override_for_path(&path)
}

/// Deterministic test hook for reparse classification (used by Gate D tag-matrix tests).
pub fn set_test_reparse_override(path: &Path, class: Option<ReparseTagClass>) {
    if let Ok(mut map) = test_reparse_override_map().lock() {
        let s = path.to_string_lossy().to_string();
        let keys = [
            normalize_test_path_key(&s),
            normalize_test_path_key(&s.replace('/', "\\")),
        ];
        match class {
            Some(c) => {
                for k in keys {
                    map.insert(k, c);
                }
            }
            None => {
                for k in keys {
                    map.remove(&k);
                }
            }
        }
    }
}

/// Return `true` if a Win32 error likely indicates the path/object is missing.
pub fn is_not_found(code: u32) -> bool {
    matches!(code, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
}

/// Return `true` if an open-for-delete error likely indicates a lock/sharing conflict.
pub fn is_lock_like_error(code: u32) -> bool {
    is_sharing_violation(code)
}

/// Retire (delete) a single Sweeper journal/log file within `logs_dir`.
///
/// Strictly scoped maintenance helper for log rotation (not a `Clean` target deletion primitive).
/// Only files whose names match Sweeper journal log naming (`clean-*.jsonl`) are eligible.
pub fn retire_sweeper_log_file(logs_dir: &Path, file_name: &str) -> Result<()> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || !is_sweeper_journal_log_name(file_name)
    {
        return Err(WinfsError::Win32 {
            context: "retire_sweeper_log_file(validate file_name)",
            code: ERROR_INVALID_PARAMETER,
        });
    }

    let dir_handle = open_for_inspection(logs_dir)?;
    let inspection = inspect_handle(&dir_handle)?;
    if (inspection.attributes & FILE_ATTRIBUTE_DIRECTORY) == 0
        || matches!(inspection.reparse, ReparseTagClass::LinkPoint)
    {
        return Err(WinfsError::Win32 {
            context: "retire_sweeper_log_file(validate logs_dir)",
            code: ERROR_INVALID_PARAMETER,
        });
    }

    let final_dir = PathBuf::from(strip_extended_prefix(&inspection.final_path));
    let target_path = final_dir.join(file_name);
    let file_handle = match open_for_inspection(&target_path) {
        Ok(h) => h,
        Err(WinfsError::Win32 { code, .. }) if is_not_found(code) => return Ok(()),
        Err(e) => return Err(e),
    };
    let file_inspection = inspect_handle(&file_handle)?;
    if (file_inspection.attributes & FILE_ATTRIBUTE_DIRECTORY) != 0
        || !matches!(file_inspection.reparse, ReparseTagClass::NotReparse)
    {
        return Err(WinfsError::Win32 {
            context: "retire_sweeper_log_file(validate target file)",
            code: ERROR_INVALID_PARAMETER,
        });
    }
    if !is_path_direct_child_of_dir(
        &strip_extended_prefix(&file_inspection.final_path),
        &final_dir,
    ) {
        return Err(WinfsError::Win32 {
            context: "retire_sweeper_log_file(validate containment)",
            code: ERROR_INVALID_PARAMETER,
        });
    }
    drop(file_handle);

    let wide = to_wide(&target_path);
    // SAFETY: `wide` is a NUL-terminated UTF-16 path buffer for a validated Sweeper-owned log file
    // directly inside the inspected logs directory. This is maintenance-only log rotation and does not
    // implement general `Clean` target deletion semantics.
    let ok = unsafe { DeleteFileW(wide.as_ptr()) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if is_not_found(err) {
            return Ok(());
        }
        return Err(WinfsError::Win32 {
            context: "DeleteFileW(retire_sweeper_log_file)",
            code: err,
        });
    }
    Ok(())
}

fn is_sweeper_journal_log_name(file_name: &str) -> bool {
    let Some(rest) = file_name.strip_prefix("clean-") else {
        return false;
    };
    let Some(stem) = rest.strip_suffix(".jsonl") else {
        return false;
    };
    if stem.is_empty() || !stem.is_ascii() {
        return false;
    }
    // Expected shape: <unix_secs>-<pid>-<nanos>
    let parts: Vec<&str> = stem.split('-').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn strip_extended_prefix(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

fn is_path_direct_child_of_dir(path: &str, dir: &Path) -> bool {
    let stripped = strip_extended_prefix(path);
    let path = Path::new(&stripped);
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(name) = path.file_name() else {
        return false;
    };
    // Best-effort lexical containment against resolved directory path.
    parent
        .to_string_lossy()
        .eq_ignore_ascii_case(&dir.to_string_lossy())
        && !name.to_string_lossy().is_empty()
}

/// Create an NTFS directory junction (mount point reparse point) at `link` targeting `target`.
///
/// Test/support helper. The created link entry is a reparse point; callers must never traverse it
/// as a directory during deletion/scanning of target contents.
///
/// # Safety and semantics
/// - `link` must not already exist as a file/directory/junction (this function creates the link directory).
/// - `target` is canonicalized and converted to an absolute NT path form for the substitute name.
#[cfg(any(test, feature = "test-support"))]
pub fn create_directory_junction(link: &Path, target: &Path) -> Result<()> {
    let target_abs = std::fs::canonicalize(target).map_err(|e| WinfsError::Win32 {
        context: "canonicalize(create_directory_junction target)",
        code: e.raw_os_error().unwrap_or(0) as u32,
    })?;
    let target_text = target_abs
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .trim_start_matches(r"\\?\UNC\")
        .to_string();
    let target_dos = if target_abs.to_string_lossy().starts_with(r"\\?\UNC\") {
        format!(r"\\{}", target_text)
    } else {
        target_text
    };
    let substitute_name = format!(r"\??\{}", target_dos);
    let print_name = target_dos;

    let mut sub_w: Vec<u16> = OsStr::new(&substitute_name).encode_wide().collect();
    sub_w.push(0);
    let mut print_w: Vec<u16> = OsStr::new(&print_name).encode_wide().collect();
    print_w.push(0);

    let sub_bytes_len = (sub_w.len() - 1) * 2;
    let print_bytes_len = (print_w.len() - 1) * 2;
    let sub_offset = 0usize;
    let print_offset = sub_bytes_len + 2; // after substitute NUL terminator
    let path_buffer_bytes = print_offset + print_bytes_len + 2;
    let reparse_data_length = 8 + path_buffer_bytes; // mount point fields + path buffer
    let total_size = 8 + reparse_data_length; // tag + data_len + reserved + data

    let mut buf = vec![0u8; total_size];
    // ReparseTag
    buf[0..4].copy_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
    // ReparseDataLength
    buf[4..6].copy_from_slice(&(reparse_data_length as u16).to_le_bytes());
    // Reserved = 0
    // SubstituteNameOffset / Length
    buf[8..10].copy_from_slice(&(sub_offset as u16).to_le_bytes());
    buf[10..12].copy_from_slice(&(sub_bytes_len as u16).to_le_bytes());
    // PrintNameOffset / Length
    buf[12..14].copy_from_slice(&(print_offset as u16).to_le_bytes());
    buf[14..16].copy_from_slice(&(print_bytes_len as u16).to_le_bytes());
    // PathBuffer: substitute + NUL + print + NUL
    let path_buf_start = 16usize;
    for (i, ch) in sub_w.iter().enumerate() {
        let off = path_buf_start + sub_offset + i * 2;
        buf[off..off + 2].copy_from_slice(&ch.to_le_bytes());
    }
    for (i, ch) in print_w.iter().enumerate() {
        let off = path_buf_start + print_offset + i * 2;
        buf[off..off + 2].copy_from_slice(&ch.to_le_bytes());
    }

    std::fs::create_dir(link).map_err(|e| WinfsError::Win32 {
        context: "CreateDirectory(create_directory_junction link)",
        code: e.raw_os_error().unwrap_or(0) as u32,
    })?;

    let wide_link = to_wide(link);
    // SAFETY: wide_link is NUL-terminated UTF-16 and valid for the call. The handle is opened on
    // the newly created directory with reparse-point access and no following of link targets. On
    // success we issue FSCTL_SET_REPARSE_POINT with a correctly sized mount-point buffer, then close
    // the handle via SafeHandle/CloseHandle.
    let handle = unsafe {
        CreateFileW(
            wide_link.as_ptr(),
            FILE_GENERIC_WRITE | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        // SAFETY: best-effort cleanup of the just-created empty directory; `wide_link` is valid NUL-terminated UTF-16.
        let _ = unsafe { RemoveDirectoryW(wide_link.as_ptr()) };
        return Err(last_error("CreateFileW(create_directory_junction)"));
    }
    let sh = SafeHandle(handle);
    let mut bytes_returned: u32 = 0;
    // SAFETY: sh is a valid open handle to the directory reparse point. uf contains a valid
    // IO_REPARSE_TAG_MOUNT_POINT reparse data buffer with lengths/offsets consistent with PathBuffer.
    let ok = unsafe {
        DeviceIoControl(
            sh.raw(),
            FSCTL_SET_REPARSE_POINT,
            buf.as_ptr() as *const _,
            buf.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        let err = last_error("DeviceIoControl(FSCTL_SET_REPARSE_POINT)");
        // SAFETY: best-effort cleanup of the directory we created for the failed junction setup.
        let _ = unsafe { RemoveDirectoryW(wide_link.as_ptr()) };
        return Err(err);
    }
    Ok(())
}

/// Set the last write time for a file or directory path (fast Win32 helper; no process spawn).
///
/// iletime is in Windows FILETIME units (100-nanosecond intervals since 1601-01-01 UTC).
#[cfg(any(test, feature = "test-support"))]
pub fn set_last_write_time(path: &Path, filetime: u64) -> Result<()> {
    let wide = to_wide(path);
    // SAFETY: NUL-terminated UTF-16 path buffer is valid for the call. Handle is opened for
    // attribute write without following reparse points and closed via SafeHandle.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(last_error("CreateFileW(set_last_write_time)"));
    }
    let sh = SafeHandle(handle);
    let ft = FILETIME {
        dwLowDateTime: (filetime & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (filetime >> 32) as u32,
    };
    // SAFETY: sh is valid and t points to a valid FILETIME struct for the duration of call.
    let ok = unsafe { SetFileTime(sh.raw(), std::ptr::null(), std::ptr::null(), &ft) };
    if ok == 0 {
        return Err(last_error("SetFileTime"));
    }
    Ok(())
}
