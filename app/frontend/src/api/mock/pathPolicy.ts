/**
 * Lexical mirror of the backend Policy rules for Custom Path validation.
 *
 * The mock has no filesystem, so the two on-disk-only checks are represented by
 * synthetic, clearly fictional path segments (never real user data):
 *   - a segment named `missing`  => path does not exist (no fallback to parent)
 *   - a segment named `linkpoint` => path is a Link Point (never followed)
 * Every other rule maps 1:1 to `sweeper-core::policy::validate_custom_path`
 * and `is_protected_for_selection`. Keep this file in sync with those rules.
 *
 * Policy comparisons are expressed over `PathSegment` domain values rather than
 * raw strings: a segment owns its ASCII case-insensitive identity, so no helper
 * needs to be handed loose strings just to compare them. The backend folds with
 * `to_ascii_uppercase`/`eq_ignore_ascii_case`, never Unicode casing (`ß` -> `SS`,
 * `K` -> `k`), so the folded identities here are ASCII-only too.
 */

/** Synthetic segment: the path does not exist on disk in the mock. */
export const MISSING_SEGMENT = 'missing';
/** Synthetic segment: the path is a Link Point in the mock. */
export const LINK_POINT_SEGMENT = 'linkpoint';

/** A single path segment as a domain value (ASCII case-insensitive by Policy rule). */
type PathSegment = {
  readonly text: string;
  /** ASCII-lowercase identity: mirrors Rust `eq_ignore_ascii_case`. */
  readonly folded: string;
  /** ASCII-uppercase identity: mirrors Rust `to_ascii_uppercase`. */
  readonly upper: string;
};
const seg = (text: string): PathSegment => ({
  text,
  folded: text.replace(/[A-Z]/g, (c) => c.toLowerCase()),
  upper: text.replace(/[a-z]/g, (c) => c.toUpperCase()),
});

/** Parsed, normalized view of a Windows path consumed by the policy checks. */
type ParsedMockPath = { readonly normalized: string; readonly segments: readonly PathSegment[] };

/** Fixed Subtree-Protected Path prefixes, matched from the drive root (drive-agnostic). */
const FIXED_SUBTREE_PROTECTED: ReadonlyArray<readonly PathSegment[]> = [
  [seg('Windows'), seg('System32')],
  [seg('Windows'), seg('WinSxS')],
  [seg('Windows'), seg('SysWOW64')],
  [seg('Windows'), seg('Boot')],
  [seg('Windows'), seg('Fonts')],
  [seg('Windows'), seg('servicing')],
  [seg('Program Files')],
  [seg('Program Files (x86)')],
  [seg('Recovery')],
  [seg('System Volume Information')],
];
/** Fixed Protected Container Roots, matched exactly at the drive root. */
const FIXED_CONTAINER_ROOTS = [seg('Windows'), seg('Users'), seg('ProgramData')] as const;
/** Well-known user data folders: subtree-protected under any profile. */
const USER_DATA_FOLDERS = [seg('Desktop'), seg('Documents'), seg('Downloads'), seg('Pictures'), seg('Videos'), seg('Music')] as const;
/** Secret / VCS folders: subtree-protected by name anywhere. */
const SECRET_SEGMENTS = [seg('.git'), seg('.ssh'), seg('.gnupg'), seg('.aws'), seg('.kube'), seg('.docker')] as const;
/** Other segment names that are subtree-protected anywhere. */
const OTHER_PROTECTED_SEGMENTS = [seg('Recovery'), seg('System Volume Information')] as const;
/** OS system files: subtree-protected by final segment. */
const OS_FILE_NAMES = [seg('pagefile.sys'), seg('hiberfil.sys'), seg('swapfile.sys')] as const;

const USERS_SEGMENT = seg('Users');
const USER_DATA_CONTAINER_SEGMENT = seg('User Data');
const LINK_POINT_VALUE = seg(LINK_POINT_SEGMENT);
const MISSING_VALUE = seg(MISSING_SEGMENT);

export type CustomPathVerdict = { path: string; allowed: boolean; reason: string };

/** ASCII case-insensitive equality of two path segments. */
const eq = (a: PathSegment, b: PathSegment): boolean => a.folded === b.folded;
const equalsAny = (value: PathSegment, options: readonly PathSegment[]): boolean => options.some((o) => eq(value, o));
const hasSegment = (segments: readonly PathSegment[], name: PathSegment): boolean => segments.some((s) => eq(s, name));

/** Parse a Windows path into drive prefix + segments, or null for invalid syntax. */
function parseMockPath(input: string): ParsedMockPath | null {
  if (!/^[A-Za-z]:[\\/]/.test(input)) return null;
  // Normalize separators once, then validate the raw structure. Empty or
  // malformed components are never filtered/trimmed into a valid-looking path.
  const replaced = input.replace(/\//g, '\\');
  const drive = replaced.slice(0, 2);
  const rest = replaced.slice(3);
  // Backend `lexical_parse` rejects empty components before normalization:
  // `C:\a\\b` and `C:\a\b\` are EmptySegment errors. A bare drive root (`C:\`)
  // is the only empty "rest" and is left to the depth guard, as before.
  if (rest !== '' && rest.split('\\').some((s) => s === '')) return null;
  // Backend rejects any ':' outside the drive position (Alternate Data Stream).
  if (replaced.indexOf(':', 2) !== -1) return null;
  const parts = rest === '' ? [] : rest.split('\\');
  const segments: readonly PathSegment[] = parts.map(seg);
  if (segments.some((s) => s.text === '.' || s.text === '..')) return null;
  // Backend `validate_segment` rejects trailing dots and spaces per segment.
  if (segments.some((s) => /[. ]$/.test(s.text))) return null;
  const normalized = segments.length
    ? [drive.toUpperCase(), ...segments.map((s) => s.text)].join('\\')
    : `${drive.toUpperCase()}\\`;
  return { normalized, segments };
}

/** Case-insensitive: does `segments` begin with the whole `prefix`? */
function startsWithCi(segments: readonly PathSegment[], prefix: readonly PathSegment[]): boolean {
  return prefix.every((p, i) => i < segments.length && eq(segments[i], p));
}

/** True if selecting `segments` would be an ancestor of a fixed protected subtree. */
function isAncestorOfFixedSubtreeProtected(segments: readonly PathSegment[]): boolean {
  return FIXED_SUBTREE_PROTECTED.some(
    (pattern) => pattern.length >= segments.length && segments.every((s, i) => eq(s, pattern[i])),
  );
}

function isOneDriveRootSegment(segment: PathSegment): boolean {
  const upper = segment.upper;
  if (!upper.startsWith('ONEDRIVE')) return false;
  const rest = upper.slice('ONEDRIVE'.length);
  return rest === '' || !/[A-Z]/.test(rest[0]);
}

function isUserProfileDataSubtree(segments: readonly PathSegment[]): boolean {
  return segments.length >= 3 && eq(segments[0], USERS_SEGMENT) && equalsAny(segments[2], USER_DATA_FOLDERS);
}

function isSubtreeProtected(segments: readonly PathSegment[]): boolean {
  if (FIXED_SUBTREE_PROTECTED.some((pattern) => startsWithCi(segments, pattern))) return true;
  if (segments.some((s) => equalsAny(s, [...SECRET_SEGMENTS, ...OTHER_PROTECTED_SEGMENTS]))) return true;
  const last = segments[segments.length - 1];
  if (last) {
    if (last.folded.startsWith('ntuser.dat') || equalsAny(last, OS_FILE_NAMES)) return true;
  }
  return isUserProfileDataSubtree(segments) || segments.some(isOneDriveRootSegment);
}

function isUnderProtectedContainerRoot(segments: readonly PathSegment[]): boolean {
  return (
    segments.length === 0 ||
    FIXED_CONTAINER_ROOTS.some((root) => startsWithCi(segments, [root])) ||
    hasSegment(segments, USER_DATA_CONTAINER_SEGMENT)
  );
}

/** Specific, human-safe reason for a path blocked by Policy, or null when allowed. */
function protectionReasonFor(segments: readonly PathSegment[]): string | null {
  if (isSubtreeProtected(segments)) return 'Protected Path: Subtree-Protected Path.';
  if (isUnderProtectedContainerRoot(segments)) return 'Protected Path: protected container root (only allowlisted subfolders are cleanable).';
  if (isAncestorOfFixedSubtreeProtected(segments)) return 'Protected Path: cannot select an ancestor of a Subtree-Protected Path.';
  return null;
}

/** String-list adapter kept for callers/tests that reason about raw segments. */
export function protectionReason(segments: readonly string[]): string | null {
  return protectionReasonFor(segments.map(seg));
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
  const protectedReason = protectionReasonFor(segments);
  if (protectedReason) return { path: normalized, allowed: false, reason: protectedReason };
  if (hasSegment(segments, LINK_POINT_VALUE)) {
    return { path: normalized, allowed: false, reason: 'Link Point: Link Points are never followed and cannot be a Custom Path.' };
  }
  if (hasSegment(segments, MISSING_VALUE)) {
    return { path: normalized, allowed: false, reason: 'Path does not exist (no fallback to parent).' };
  }
  return { path: normalized, allowed: true, reason: 'ok' };
}
