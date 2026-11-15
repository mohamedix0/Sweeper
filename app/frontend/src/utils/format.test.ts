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
    expect(formatBytes('0')).toBe('0 B');
    expect(formatBytes('512')).toBe('512 B');
    expect(formatBytes('1024')).toBe('1 KB');
    expect(formatBytes('1048576')).toBe('1 MB');
    expect(formatBytes('7340032')).toContain('MB');
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
    expect(relativeAge(secsAgo(0), NOW)).toBe('just now');
    expect(relativeAge(secsAgo(90_000), NOW)).toBe('just now');
    expect(relativeAge(secsAgo(4 * 3_600_000), NOW)).toBe('4h ago');
    expect(relativeAge(secsAgo(4 * 86_400_000), NOW)).toBe('4d ago');
    expect(relativeAge(secsAgo(12 * 86_400_000), NOW)).toBe('12d ago');
    expect(relativeAge(secsAgo(30 * 86_400_000), NOW)).toBe('1mo ago');
    expect(relativeAge(secsAgo(60 * 86_400_000), NOW)).toBe('2mo ago');
    expect(relativeAge(secsAgo(359 * 86_400_000), NOW)).toBe('11mo ago');
    expect(relativeAge(secsAgo(360 * 86_400_000), NOW)).toBe('12mo ago');
    expect(relativeAge(secsAgo(364 * 86_400_000), NOW)).toBe('12mo ago');
    expect(relativeAge(secsAgo(365 * 86_400_000), NOW)).toBe('1y ago');
    expect(relativeAge(secsAgo(400 * 86_400_000), NOW)).toBe('1y ago');
  });

  it('treats future timestamps as just now and unsafe values as em dash', () => {
    expect(relativeAge(String(Math.floor(NOW / 1000) + 60), NOW)).toBe('just now');
    expect(relativeAge(null, NOW)).toBe('—');
    expect(relativeAge(undefined, NOW)).toBe('—');
    expect(relativeAge('9007199254740993', NOW)).toBe('—');
    expect(relativeAge('not-a-number', NOW)).toBe('—');
  });
});

describe('normalizeDisplayPath and middleTruncatePath', () => {
  it('preserves UNC and extended-length prefixes while collapsing separators elsewhere', () => {
    expect(normalizeDisplayPath('\\\\server\\share\\a\\\\b\\c')).toBe('\\\\server\\share\\a\\b\\c');
    expect(normalizeDisplayPath('\\\\?\\C:\\temp\\\\logs\\app.log')).toBe('\\\\?\\C:\\temp\\logs\\app.log');
    expect(normalizeDisplayPath('\\\\?\\UNC\\server\\share\\\\dir\\file.txt')).toBe('\\\\?\\UNC\\server\\share\\dir\\file.txt');
    expect(middleTruncatePath('\\\\server\\share\\a\\\\b\\c', 200)).toBe('\\\\server\\share\\a\\b\\c');
    expect(middleTruncatePath('\\\\?\\C:\\temp\\\\logs\\app.log', 200)).toBe('\\\\?\\C:\\temp\\logs\\app.log');
  });
});
