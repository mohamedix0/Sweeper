/**
 * Lexical mirror of the backend Policy rules for Custom Path validation.
 *
 * The mock has no filesystem, so the two on-disk-only checks are represented by
 * synthetic, clearly fictional path segments (never real user data):
 *   - a segment named `missing`  => path does not exist (no fallback to parent)
 *   - a segment named `linkpoint` => path is a Link Point (never followed)
 * Every lexical rule maps 1:1 to `sweeper-core::paths::lexical_parse` (via
 * `validate_custom_path`/`validate_custom_path_lexical`) and to
 * `sweeper-core::policy::is_protected_for_selection`, including the reserved
 * DOS device names, disallowed Unicode, and the 32,767-byte / 255-byte-per-
 * segment length caps. Keep this file in sync with those rules.
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

/** Raw, separator-normalized path text split into drive prefix and post-drive remainder. */
type RawPath = { readonly replaced: string; readonly rest: string };

/** Backend `lexical_parse` syntax checks that must run before normalization. */
function hasInvalidRawSyntax({ replaced, rest }: RawPath): boolean {
  // Empty components are EmptySegment errors (`C:\a\\b`, `C:\a\b\`). A bare
  // drive root (`C:\`) is the only empty "rest" and is left to the depth guard.
  if (rest !== '' && rest.split('\\').some((s) => s === '')) return true;
  // Any ':' outside the drive position is an Alternate Data Stream.
  return replaced.indexOf(':', 2) !== -1;
}

/** UTF-8 byte length, mirroring Rust `str::len()` used by the backend length caps. */
function utf8ByteLength(text: string): number {
  return new TextEncoder().encode(text).length;
}

/**
 * Backend `check_disallowed_unicode`: NUL/control characters (Unicode Cc),
 * bidi overrides/isolates and fullwidth separator tricks are all rejected.
 * The ranges mirror `sweeper-core::paths::check_disallowed_unicode` exactly.
 */
const DISALLOWED_UNICODE = /[\p{Cc}\u061C\u200E\u200F\u202A-\u202E\u2066-\u2069\uFEFF\uFF0F\uFF3C\uFF1A\uFF0E]/u;

/** Backend `is_reserved_dos_name` base names. */
const RESERVED_DOS_NAMES: ReadonlySet<string> = new Set([
  'CON', 'PRN', 'AUX', 'NUL',
  'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9',
  'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9',
]);

/** Superscript digits normalized to ASCII by backend `is_reserved_dos_name_unicode_variant`. */
const SUPERSCRIPT_DIGITS: Readonly<Record<string, string>> = {
  '¹': '1', '²': '2', '³': '3', '⁴': '4', '⁵': '5', '⁶': '6', '⁷': '7', '⁸': '8', '⁹': '9',
};

/**
 * Backend `validate_segment` reserved-DOS-name check: uppercase base name before
 * the optional extension, with superscript digit variants normalized to ASCII.
 */
function isReservedDosSegment(segment: PathSegment): boolean {
  const base = segment.upper.split('.')[0];
  const normalized = base.replace(/[¹²³⁴⁵⁶⁷⁸⁹]/g, (c) => SUPERSCRIPT_DIGITS[c]);
  return RESERVED_DOS_NAMES.has(normalized);
}

/**
 * Backend `validate_segment` plus the per-segment length cap from `lexical_parse`:
 * '.', '..', trailing dots/spaces, over-long segments and reserved DOS names.
 */
function hasInvalidSegment(segments: readonly PathSegment[]): boolean {
  return segments.some(
    (s) =>
      s.text === '.' ||
      s.text === '..' ||
      /[. ]$/.test(s.text) ||
      utf8ByteLength(s.text) > 255 ||
      isReservedDosSegment(s),
  );
}

/** Parse a Windows path into drive prefix + segments, or null for invalid syntax. */
function parseMockPath(input: string): ParsedMockPath | null {
  // Backend `lexical_parse` rejects over-long inputs and disallowed Unicode
  // before any structural/normalization work.
  if (utf8ByteLength(input) > 32_767) return null;
  if (DISALLOWED_UNICODE.test(input)) return null;
  if (!/^[A-Za-z]:[\\/]/.test(input)) return null;
  // Normalize separators once, then validate the raw structure. Empty or
  // malformed components are never filtered/trimmed into a valid-looking path.
  const replaced = input.replace(/\//g, '\\');
  const drive = replaced.slice(0, 2);
  const rest = replaced.slice(3);
  if (hasInvalidRawSyntax({ replaced, rest })) return null;
  const parts = rest === '' ? [] : rest.split('\\');
  const segments: readonly PathSegment[] = parts.map(seg);
  if (hasInvalidSegment(segments)) return null;
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
