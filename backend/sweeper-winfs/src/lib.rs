//! Raw Windows NTFS operations (read-only handle inspection).
//! Sole crate permitted to contain `unsafe`. No deletion primitives.

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use thiserror::Error;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ENVVAR_NOT_FOUND, ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES,
    ERROR_PATH_NOT_FOUND, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FIND_FIRST_EX_LARGE_FETCH, OPEN_EXISTING,
    WIN32_FIND_DATAW,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Environment::GetEnvironmentVariableW;
use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
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

/// Errors returned by `sweeper-winfs` read-only operations.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Open a file or directory handle for read-only inspection without following reparse points.
///
/// Uses `FILE_FLAG_OPEN_REPARSE_POINT` and `FILE_FLAG_BACKUP_SEMANTICS` (for directories)
/// with shared read/write/delete access and `OPEN_EXISTING`.
pub fn open_for_inspection(path: &Path) -> Result<SafeHandle> {
    let wide = to_wide(path);
    let flags = FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS;
    // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that lives for the duration of the call.
    // We request read attribute/access via share flags and no write/delete disposition; opening is
    // strictly read-only. `FILE_FLAG_OPEN_REPARSE_POINT` ensures link points are not followed.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0, // read-only inspection: no data access required for metadata APIs used here
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

/// Resolve canonical path for an opened handle via `GetFinalPathNameByHandleW`.
pub fn get_final_path(handle: &SafeHandle) -> Result<String> {
    // VOLUME_NAME_DOS = 0x0
    const VOLUME_NAME_DOS: u32 = 0;
    // FILE_NAME_NORMALIZED = 0x0
    const FILE_NAME_NORMALIZED: u32 = 0;
    let flags = VOLUME_NAME_DOS | FILE_NAME_NORMALIZED;
    // SAFETY: passing a valid handle and null buffer with size 0 is defined to query required
    // length (including NUL). `handle` remains valid for the call.
    let needed = unsafe { GetFinalPathNameByHandleW(handle.raw(), std::ptr::null_mut(), 0, flags) };
    if needed == 0 {
        return Err(last_error("GetFinalPathNameByHandleW(len)"));
    }
    let mut buf = vec![0u16; needed as usize];
    // SAFETY: `buf` is writable for `needed` u16 elements and `handle` is valid. The API writes a
    // NUL-terminated path string on success.
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
    // SAFETY: zeroed `BY_HANDLE_FILE_INFORMATION` is valid to write into; `handle` must be valid
    // and open, which `SafeHandle` guarantees while alive.
    let mut info: windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION =
        unsafe { std::mem::zeroed() };
    // SAFETY: `info` points to writable memory of correct size for the duration of the call.
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

/// Read-only handle-aware directory enumeration.
///
/// Uses `FindFirstFileExW` with `FIND_FIRST_EX_LARGE_FETCH`. Never follows `Link Point`s:
/// entries are returned as-is with reparse classification so callers can avoid traversal.
pub fn read_dir_entries(dir: &Path) -> Result<Vec<DirEntry>> {
    let pattern = dir.join("*");
    let wide = to_wide(&pattern);
    // SAFETY: zeroed `WIN32_FIND_DATAW` is a valid output buffer for FindFirstFileExW/Next.
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    // SAFETY: `wide` is NUL-terminated and lives for the call; `data` is writable. We request basic
    // info with large fetch for efficiency. Enumeration is read-only and does not follow reparse
    // targets (it only lists entries in `dir`).
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
        // SAFETY: `handle` is a valid find handle from FindFirstFileExW and `data` is writable.
        // Advancing enumeration is read-only.
        let ok = unsafe { FindNextFileW(handle, &mut data) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            // SAFETY: closing the valid find handle once after enumeration completes/fails.
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
        // Conservative heuristic for cldflt/cloud placeholder families (0x9000_001A-style).
        other if (other & 0xF000_FFFF) == 0x9000_001A => ReparseTagClass::CloudPlaceholder,
        other => ReparseTagClass::OtherReparse(other),
    }
}

/// Inspect reparse tag for an opened handle via `FSCTL_GET_REPARSE_POINT`.
///
/// Returns `ReparseTagClass::NotReparse` if the handle/path is not a reparse point.
pub fn inspect_reparse_point(handle: &SafeHandle) -> Result<ReparseTagClass> {
    // Buffer large enough for common reparse data; MAXIMUM_REPARSE_DATA_BUFFER_SIZE is 16384.
    let mut buf = vec![0u8; 16 * 1024];
    let mut bytes_returned: u32 = 0;
    // SAFETY: `handle` is valid. `buf` is writable for its full length and `bytes_returned` points
    // to writable u32. DeviceIoControl with FSCTL_GET_REPARSE_POINT reads reparse metadata only.
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
        // ERROR_NOT_A_REPARSE_POINT = 4390
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
    // SAFETY: buffer contains at least 4 bytes on success; first field of reparse data is tag.
    let tag = unsafe { *(buf.as_ptr() as *const u32) };
    Ok(classify_reparse_tag(tag))
}

/// Read-only Known Folder path resolution helper using `SHGetKnownFolderPath`.
///
/// The caller provides a known folder `GUID` (e.g. `FOLDERID_*` constants). This is a read-only
/// helper for Phase 1B; higher-level enum mapping lives in `sweeper-knowledge`.
pub fn known_folder_path(folder_id: &GUID) -> Result<PathBuf> {
    let mut path_ptr: *mut u16 = std::ptr::null_mut();
    // SAFETY: `folder_id` points to a valid `GUID` for the duration of the call. `path_ptr` is a
    // writable out parameter; on success SHGetKnownFolderPath allocates a NUL-terminated UTF-16
    // string with CoTaskMemAlloc that we must free via CoTaskMemFree.
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
    // SAFETY: on success `path_ptr` is a valid NUL-terminated UTF-16 string allocated by COM task
    // memory allocator and remains valid until freed.
    let path = unsafe {
        let mut len = 0usize;
        while *path_ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(path_ptr, len);
        let os = OsString::from_wide(slice);
        PathBuf::from(os)
    };
    // SAFETY: `path_ptr` was allocated by SHGetKnownFolderPath/CoTaskMemAlloc on success and must
    // be released exactly once with CoTaskMemFree.
    unsafe { CoTaskMemFree(path_ptr as *const _) };
    Ok(path)
}

/// Read-only environment variable helper using `GetEnvironmentVariableW`.
pub fn env_var(name: &str) -> Option<String> {
    let wide_name = to_wide_os(OsStr::new(name));
    // SAFETY: query required size with null buffer/0; `wide_name` is NUL-terminated.
    let needed = unsafe { GetEnvironmentVariableW(wide_name.as_ptr(), std::ptr::null_mut(), 0) };
    if needed == 0 {
        let err = unsafe { GetLastError() };
        if err == ERROR_ENVVAR_NOT_FOUND {
            return None;
        }
        return None;
    }
    let mut buf = vec![0u16; needed as usize];
    // SAFETY: `buf` is writable for `needed` elements; API writes value including NUL on success.
    let written = unsafe { GetEnvironmentVariableW(wide_name.as_ptr(), buf.as_mut_ptr(), needed) };
    if written == 0 {
        return None;
    }
    let slice = &buf[..written as usize];
    String::from_utf16(slice).ok()
}

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
    // SAFETY: `GetLastError` reads thread-local last-error state and has no preconditions.
    let code = unsafe { GetLastError() };
    WinfsError::Win32 { context, code }
}
