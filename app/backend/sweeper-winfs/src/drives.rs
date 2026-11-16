//! Read-only enumeration of local fixed drives.
//!
//! This module owns the Win32 drive-query calls. It opens no handles, follows no reparse
//! points and mutates nothing, so it cannot traverse a Link Point or hydrate a cloud
//! placeholder. Callers remain responsible for applying their own Protected Path and Link
//! Point policy to any path they derive from a returned drive letter (ADR 0001 / 0003).

use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

/// `DRIVE_FIXED` return value of `GetDriveTypeW` (not exposed by the enabled windows-sys features).
const DRIVE_FIXED: u32 = 3;

/// Returns the letters of all local fixed drives currently present (e.g. `['C', 'D']`).
///
/// `GetLogicalDrives` supplies the present-drive bitmask and `GetDriveTypeW` filters it
/// down to `DRIVE_FIXED` only, excluding removable media, network shares, CD-ROM and RAM
/// drives. This is a read-only query: no filesystem object is opened and nothing is
/// mutated, so no Link Point is followed and no cloud placeholder is hydrated.
pub fn fixed_drive_letters() -> Vec<char> {
    // SAFETY: `GetLogicalDrives` takes no arguments and has no preconditions. It returns a
    // bitmask (bit 0 = drive A) and touches no caller-owned memory.
    let mask = unsafe { GetLogicalDrives() };
    logical_drive_letters(mask)
        .into_iter()
        .filter(|letter| {
            let root = format!("{letter}:\\");
            let wide = to_wide(&root);
            // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that stays alive for this call.
            // `GetDriveTypeW` only reads that root-path string and returns a `DRIVE_*` constant;
            // it performs no filesystem mutation and does not traverse reparse points.
            let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
            is_fixed_drive_type(drive_type)
        })
        .collect()
}

/// Pure helper: decode a `GetLogicalDrives` bitmask into drive letters (bit 0 = `A`).
fn logical_drive_letters(mask: u32) -> Vec<char> {
    (0u8..26)
        .filter(|i| (mask & (1u32 << *i)) != 0)
        .map(|i| (b'A' + i) as char)
        .collect()
}

/// Pure helper: true only for the `DRIVE_FIXED` result of `GetDriveTypeW`.
fn is_fixed_drive_type(drive_type: u32) -> bool {
    drive_type == DRIVE_FIXED
}

fn to_wide(root: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = root.encode_utf16().collect();
    wide.push(0);
    wide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_drive_letters_decodes_mask() {
        assert!(logical_drive_letters(0).is_empty());
        assert_eq!(logical_drive_letters(1 << 0), vec!['A']);
        assert_eq!(logical_drive_letters((1 << 2) | (1 << 3)), vec!['C', 'D']);
        assert_eq!(logical_drive_letters(1 << 25), vec!['Z']);
    }

    #[test]
    fn is_fixed_drive_type_accepts_only_drive_fixed() {
        assert!(is_fixed_drive_type(DRIVE_FIXED));
        for other in [0u32, 1, 2, 4, 5, 6] {
            assert!(
                !is_fixed_drive_type(other),
                "drive type {other} must not be treated as fixed"
            );
        }
    }
}
