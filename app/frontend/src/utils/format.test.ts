import { describe, expect, it } from 'vitest';
import { formatBytes, formatUnixSeconds, middleTruncatePath, normalizeDisplayPath, relativeAge, sumDecimalStrings, toBigInt } from './format';

describe('format utils (BigInt-safe)', () => {
  it('parses decimal strings as BigInt', () => {
    expect(toBigInt('0')).toBe(0n);
    expect(toBigInt('9007199254740993')).toBe(9007199254740993n);
  });

  it('sums decimal strings using BigInt', () => {
    expect(sumDecimalStrings(['1048576', '2048', '5242880'])).toBe(6293504n);
    expect(sumDecimalStrings(['9007199254740991', '2'])).toBe(9007199254740993n);
  });

  it('formats bytes with units', () => {
    const cases: Array<[string, string, boolean]> = [['0', '0 B', false], ['512', '512 B', false], ['1024', '1 KB', false], ['1048576', '1 MB', false], ['7340032', 'MB', true]];
    for (const [input, out, contains] of cases) {
      const actual = formatBytes(input);
      if (contains) expect(actual).toContain(out);
      else expect(actual).toBe(out);
    }
  });

  it('handles very large byte counts without Number precision loss', () => {
    const big = '9007199254740993';
    expect(formatBytes(big)).toContain('PB');
  });

  it('range-checks BigInt before Number conversion for unix seconds', () => {
    expect(formatUnixSeconds(null)).toBe('—');
    expect(formatUnixSeconds('0')).toBe(new Date(0).toLocaleString());
    expect(formatUnixSeconds('9007199254740993')).toBe('9007199254740993');
  });
});

describe('relativeAge (fixed now)', () => {
  const NOW = Date.parse('2026-11-15T12:00:00Z');
  const secsAgo = (ms: number) => String(Math.floor((NOW - ms) / 1000));

  it('renders human relative ages from unix seconds', () => {
    const cases: Array<[string, string]> = [
      [secsAgo(0), 'just now'], [secsAgo(90_000), 'just now'], [secsAgo(4 * 3_600_000), '4h ago'],
      [secsAgo(4 * 86_400_000), '4d ago'], [secsAgo(12 * 86_400_000), '12d ago'], [secsAgo(30 * 86_400_000), '1mo ago'],
      [secsAgo(60 * 86_400_000), '2mo ago'], [secsAgo(359 * 86_400_000), '11mo ago'], [secsAgo(360 * 86_400_000), '12mo ago'],
      [secsAgo(364 * 86_400_000), '12mo ago'], [secsAgo(365 * 86_400_000), '1y ago'], [secsAgo(400 * 86_400_000), '1y ago'],
    ];
    for (const [input, out] of cases) expect(relativeAge(input, NOW)).toBe(out);
  });

  it('treats future timestamps as just now and unsafe values as em dash', () => {
    const cases: Array<[string | null | undefined, string]> = [[String(Math.floor(NOW / 1000) + 60), 'just now'], [null, '—'], [undefined, '—'], ['9007199254740993', '—'], ['not-a-number', '—']];
    for (const [input, out] of cases) expect(relativeAge(input, NOW)).toBe(out);
  });
});

describe('normalizeDisplayPath and middleTruncatePath', () => {
  it('preserves UNC and extended-length prefixes while collapsing separators elsewhere', () => {
    const paths = [['\\\\server\\share\\a\\\\b\\c', '\\\\server\\share\\a\\b\\c'], ['\\\\?\\C:\\temp\\\\logs\\app.log', '\\\\?\\C:\\temp\\logs\\app.log'], ['\\\\?\\UNC\\server\\share\\\\dir\\file.txt', '\\\\?\\UNC\\server\\share\\dir\\file.txt']] as const;
    for (const [input, expected] of paths) expect(normalizeDisplayPath(input)).toBe(expected);
    for (const [input, expected] of paths.slice(0, 2)) expect(middleTruncatePath(input, 200)).toBe(expected);
  });
});
