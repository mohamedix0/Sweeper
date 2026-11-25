import { describe, expect, it } from 'vitest';
import { evaluateCustomPath } from './pathPolicy';

const verdict = (path: string) => evaluateCustomPath(path);

/**
 * Regression coverage for the lexical rules that the mock previously omitted
 * (audit finding T1). Each case mirrors a `sweeper-core::paths` rule enforced by
 * `lexical_parse` / `validate_segment` / `check_disallowed_unicode`; the mock must
 * reject the same inputs so dev/demo E2E cannot drift from production Policy.
 */
describe('mock Custom Path mirrors backend lexical validation added for T1', () => {
  it('rejects reserved DOS device names (base before extension, ASCII case, superscript variants)', () => {
    // Backend `validate_segment` -> `is_reserved_dos_name` rejects the base name
    // before the optional extension, case-insensitively, plus superscript digits.
    for (const path of [
      'C:\\Temp\\CON\\foo', // the audit's trigger input
      'C:\\Temp\\con\\foo',
      'C:\\Temp\\CON.txt\\foo',
      'C:\\Temp\\nul\\foo',
      'C:\\Temp\\COM1.log\\foo',
      'C:\\Temp\\LPT9\\foo',
      'C:\\Temp\\COM¹\\foo', // U+00B9 superscript one -> COM1
      'C:\\Temp\\LPT²\\foo', // U+00B2 superscript two -> LPT2
    ]) {
      expect(verdict(path).allowed, path).toBe(false);
    }
  });

  it('rejects disallowed Unicode (bidi override, fullwidth separators, controls)', () => {
    // Backend `check_disallowed_unicode` rejects bidi overrides/isolates,
    // fullwidth slash/backslash/colon/dot and control characters anywhere.
    for (const path of [
      'C:\\Temp\\Foo\u202EBar\\Baz', // U+202E RIGHT-TO-LEFT OVERRIDE in a segment
      'C:\\Temp\\\u2066hidden\u2069\\Baz', // U+2066/U+2069 isolate pair
      'C:\\Temp\\／fullwidth\\Baz', // U+FF0F fullwidth solidus
      'C:\\Temp\\：stream\\Baz', // U+FF1A fullwidth colon
      'C:\\Temp\\bad\u0000name\\Baz', // NUL byte
      'C:\\Temp\\bad\u007Fname\\Baz', // DEL control
    ]) {
      expect(verdict(path).allowed, path).toBe(false);
    }
  });

  it('rejects inputs over the 32,767-byte cap even when every segment is within 255 bytes', () => {
    // Isolates the whole-input cap from the per-segment cap: 130 segments of 255
    // ASCII bytes put the total well over 32,767 while no single segment does.
    const segments = Array.from({ length: 130 }, () => 'a'.repeat(255));
    const path = `C:\\${segments.join('\\')}`;
    expect(verdict(path).allowed).toBe(false);
  });

  it('rejects a single segment over the 255-byte cap', () => {
    // Backend `lexical_parse` rejects `raw_seg.len() > 255` (255/segment).
    const path = `C:\\Temp\\${'a'.repeat(256)}\\Logs`;
    expect(verdict(path).allowed).toBe(false);
  });

  it('still accepts benign near-boundary inputs the rules must not over-reject', () => {
    // A reserved-name look-alike, a benign non-ASCII letter and a segment at
    // exactly 255 bytes are all valid and must remain allowed.
    expect(verdict('C:\\Temp\\CONSOLE\\foo').allowed).toBe(true);
    expect(verdict('C:\\Temp\\café\\foo').allowed).toBe(true);
    expect(verdict(`C:\\Temp\\${'a'.repeat(255)}\\Logs`).allowed).toBe(true);
  });
});
