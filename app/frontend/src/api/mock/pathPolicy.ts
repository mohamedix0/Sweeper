/**
 * Lexical mirror of the backend Policy rules for Custom Path validation.
 *
 * The mock has no filesystem, so the two on-disk-only checks are represented by
 * synthetic, clearly fictional path segments (never real user data):
 *   - a segment named `missing`  => path does not exist (no fallback to parent)
 *   - a segment named `linkpoint` => path is a Link Point (never followed)
 * Every other rule maps 1:1 to `sweeper-core::policy::validate_custom_path`
 * and `is_protected_for_selection`. Keep this file in sync with those rules.
 */

/** Synthetic segment: the path does not exist on disk in the mock. */
export const MISSING_SEGMENT = 'missing';
/** Synthetic segment: the path is a Link Point in the mock. */
export const LINK_POINT_SEGMENT = 'linkpoint';

/** Fixed Subtree-Protected Path prefixes, matched from the drive root (drive-agnostic). */
const FIXED_SUBTREE_PROTECTED: ReadonlyArray<readonly string[]> = [
  ['Windows', 'System32'],
  ['Windows', 'WinSxS'],
  ['Windows', 'SysWOW64'],
  ['Windows', 'Boot'],
  ['Windows', 'Fonts'],
  ['Windows', 'servicing'],
  ['Program Files'],
  ['Program Files (x86)'],
  ['Recovery'],
  ['System Volume Information'],
];
/** Fixed Protected Container Roots, matched exactly at the drive root. */
const FIXED_CONTAINER_ROOTS = ['Windows', 'Users', 'ProgramData'] as const;
/** Well-known user data folders: subtree-protected under any profile. */
const USER_DATA_FOLDERS = ['Desktop', 'Documents', 'Downloads', 'Pictures', 'Videos', 'Music'] as const;
/** Secret / VCS folders: subtree-protected by name anywhere. */
const SECRET_SEGMENTS = ['.git', '.ssh', '.gnupg', '.aws', '.kube', '.docker'] as const;
/** OS system files: subtree-protected by final segment. */
const OS_FILE_NAMES = ['pagefile.sys', 'hiberfil.sys', 'swapfile.sys'] as const;

export type CustomPathVerdict = { path: string; allowed: boolean; reason: string };
type ParsedMockPath = { normalized: string; segments: string[] };

const eq = (a: string, b: string): boolean => a.toLowerCase() === b.toLowerCase();
const equalsAny = (value: string, options: readonly string[]): boolean => options.some((o) => eq(value, o));
const hasSegment = (segments: readonly string[], name: string): boolean => segments.some((s) => eq(s, name));

/** Normalize a Windows path into drive prefix + segments, or null for invalid syntax. */
function parseMockPath(input: string): ParsedMockPath | null {
  if (!/^[A-Za-z]:[\\/]/.test(input)) return null;
  const parts = input.replace(/\//g, '\\').replace(/\\+$/, '').split('\\').filter(Boolean);
  const drive = parts.shift();
  if (!drive) return null;
  const segments = parts;
  if (segments.some((s) => s === '.' || s === '..')) return null;
  const normalized = segments.length ? [drive.toUpperCase(), ...segments].join('\\') : `${drive.toUpperCase()}\\`;
  return { normalized, segments };
}

/** Case-insensitive: does `segments` begin with the whole `prefix`? */
function startsWithCi(segments: readonly string[], prefix: readonly string[]): boolean {
  return prefix.every((seg, i) => i < segments.length && eq(segments[i], seg));
}

/** True if selecting `segments` would be an ancestor of a fixed protected subtree. */
function isAncestorOfFixedSubtreeProtected(segments: readonly string[]): boolean {
  return FIXED_SUBTREE_PROTECTED.some(
    (pattern) => pattern.length >= segments.length && segments.every((seg, i) => eq(seg, pattern[i])),
  );
}

function isOneDriveRootSegment(segment: string): boolean {
  const upper = segment.toUpperCase();
  if (!upper.startsWith('ONEDRIVE')) return false;
  const rest = upper.slice('ONEDRIVE'.length);
  return rest === '' || !/[A-Z]/.test(rest[0]);
}

function isUserProfileDataSubtree(segments: readonly string[]): boolean {
  return segments.length >= 3 && eq(segments[0], 'Users') && equalsAny(segments[2], USER_DATA_FOLDERS);
}

function isSubtreeProtected(segments: readonly string[]): boolean {
  if (FIXED_SUBTREE_PROTECTED.some((pattern) => startsWithCi(segments, pattern))) return true;
  if (segments.some((s) => equalsAny(s, [...SECRET_SEGMENTS, 'Recovery', 'System Volume Information']))) return true;
  const last = segments[segments.length - 1];
  if (last) {
    const upper = last.toUpperCase();
    if (upper.startsWith('NTUSER.DAT') || equalsAny(upper, OS_FILE_NAMES.map((n) => n.toUpperCase()))) return true;
  }
  return isUserProfileDataSubtree(segments) || segments.some(isOneDriveRootSegment);
}

function isUnderProtectedContainerRoot(segments: readonly string[]): boolean {
  return (
    segments.length === 0 ||
    FIXED_CONTAINER_ROOTS.some((root) => startsWithCi(segments, [root])) ||
    hasSegment(segments, 'User Data')
  );
}

/** Specific, human-safe reason for a path blocked by Policy, or null when allowed. */
export function protectionReason(segments: readonly string[]): string | null {
  if (isSubtreeProtected(segments)) return 'Protected Path: Subtree-Protected Path.';
  if (isUnderProtectedContainerRoot(segments)) return 'Protected Path: protected container root (only allowlisted subfolders are cleanable).';
  if (isAncestorOfFixedSubtreeProtected(segments)) return 'Protected Path: cannot select an ancestor of a Subtree-Protected Path.';
  return null;
}

/** Evaluate a non-empty path against the mirrored Policy rules. */
export function evaluateCustomPath(input: string): CustomPathVerdict {
  const parsed = parseMockPath(input);
  if (!parsed) {
    return { path: input, allowed: false, reason: 'Path must be an absolute drive path without traversal segments.' };
  }
  const { normalized, segments } = parsed;
  // A Custom Path only ever cleans a folder's contents, so it must be deep enough
  // to exclude drive roots and top-level containers (backend: depth >= 3 segments).
  if (segments.length < 3) {
    return { path: normalized, allowed: false, reason: 'Custom Path must have depth >= 3 segments (only folder contents are cleaned).' };
  }
  const protectedReason = protectionReason(segments);
  if (protectedReason) return { path: normalized, allowed: false, reason: protectedReason };
  if (hasSegment(segments, LINK_POINT_SEGMENT)) {
    return { path: normalized, allowed: false, reason: 'Link Point: Link Points are never followed and cannot be a Custom Path.' };
  }
  if (hasSegment(segments, MISSING_SEGMENT)) {
    return { path: normalized, allowed: false, reason: 'Path does not exist (no fallback to parent).' };
  }
  return { path: normalized, allowed: true, reason: 'ok' };
}
