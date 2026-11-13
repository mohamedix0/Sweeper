import { describe, expect, it } from 'vitest';
import { formatBytes, formatUnixSeconds, sumDecimalStrings, toBigInt } from './format';

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
