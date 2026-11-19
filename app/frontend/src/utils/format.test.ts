import { describe, expect, it } from 'vitest';
import { formatBytes, formatUnixSeconds, middleTruncatePath, normalizeDisplayPath, pluralize, relativeAge, sumDecimalStrings, toBigInt } from './format';

interface BigIntCase { input: string; expected: bigint }
interface SumCase { values: Array<string>; expected: bigint }
interface BytesCase { input: string; expected?: string; contains?: string }
interface UnixCase { secs: string | null; expected?: string; fromEpoch?: boolean }
interface RelativeCase { secs: string | null | undefined; expected: string }
interface PathCase { input: string; expected: string }

const NOW = Date.parse('2026-11-15T12:00:00Z');

function secsAgo(args: { ms: number }): string {
  return String(Math.floor((NOW - args.ms) / 1000));
}

function expectBigIntCases(cases: BigIntCase[]) {
  for (const item of cases) expect(toBigInt({ value: item.input })).toBe(item.expected);
}

function expectSumCases(cases: SumCase[]) {
  for (const item of cases) expect(sumDecimalStrings({ values: item.values })).toBe(item.expected);
}

function assertBytesCase(item: BytesCase) {
  const actual = formatBytes({ bytes: item.input });
  if (item.contains) expect(actual).toContain(item.contains);
  if (item.expected) expect(actual).toBe(item.expected);
}

function expectBytesCases(cases: BytesCase[]) {
  for (const item of cases) assertBytesCase(item);
}

function assertUnixCase(item: UnixCase) {
  const actual = formatUnixSeconds({ secs: item.secs });
  if (item.fromEpoch) expect(actual).toBe(new Date(0).toLocaleString());
  if (item.expected) expect(actual).toBe(item.expected);
}

function expectUnixCases(cases: UnixCase[]) {
  for (const item of cases) assertUnixCase(item);
}

function expectRelativeCases(cases: RelativeCase[]) {
  for (const item of cases) expect(relativeAge({ secs: item.secs, nowMs: NOW })).toBe(item.expected);
}

function expectNormalizeCases(cases: PathCase[]) {
  for (const item of cases) expect(normalizeDisplayPath({ p: item.input })).toBe(item.expected);
}

function expectTruncateCases(cases: PathCase[]) {
  for (const item of cases) expect(middleTruncatePath({ path: item.input, maxLen: 200 })).toBe(item.expected);
}

describe('format utils (BigInt-safe)', () => {
  it('parses decimal strings as BigInt', () => {
    expectBigIntCases([{ input: '0', expected: 0n }, { input: '9007199254740993', expected: 9007199254740993n }]);
  });

  it('sums decimal strings using BigInt', () => {
    expectSumCases([{ values: ['1048576', '2048', '5242880'], expected: 6293504n }, { values: ['9007199254740991', '2'], expected: 9007199254740993n }]);
  });

  it('formats bytes with units', () => {
    expectBytesCases([{ input: '0', expected: '0 B' }, { input: '512', expected: '512 B' }, { input: '1024', expected: '1 KB' }, { input: '1048576', expected: '1 MB' }, { input: '7340032', contains: 'MB' }]);
  });

  it('handles very large byte counts without Number precision loss', () => {
    expect(formatBytes({ bytes: '9007199254740993' })).toContain('PB');
  });

  it('range-checks BigInt before Number conversion for unix seconds', () => {
    expectUnixCases([{ secs: null, expected: '\u2014' }, { secs: '0', fromEpoch: true }, { secs: '9007199254740993', expected: '9007199254740993' }]);
  });

  it('pluralizes labels', () => {
    expect(pluralize({ count: 1, singular: 'item' })).toBe('1 item');
    expect(pluralize({ count: 2, singular: 'item' })).toBe('2 items');
  });
});

describe('relativeAge (fixed now)', () => {
  it('renders human relative ages from unix seconds', () => {
    expectRelativeCases([
      { secs: secsAgo({ ms: 0 }), expected: 'just now' }, { secs: secsAgo({ ms: 90_000 }), expected: 'just now' },
      { secs: secsAgo({ ms: 4 * 3_600_000 }), expected: '4h ago' }, { secs: secsAgo({ ms: 4 * 86_400_000 }), expected: '4d ago' },
      { secs: secsAgo({ ms: 12 * 86_400_000 }), expected: '12d ago' }, { secs: secsAgo({ ms: 30 * 86_400_000 }), expected: '1mo ago' },
      { secs: secsAgo({ ms: 60 * 86_400_000 }), expected: '2mo ago' }, { secs: secsAgo({ ms: 359 * 86_400_000 }), expected: '11mo ago' },
      { secs: secsAgo({ ms: 360 * 86_400_000 }), expected: '12mo ago' }, { secs: secsAgo({ ms: 364 * 86_400_000 }), expected: '12mo ago' },
      { secs: secsAgo({ ms: 365 * 86_400_000 }), expected: '1y ago' }, { secs: secsAgo({ ms: 400 * 86_400_000 }), expected: '1y ago' },
    ]);
  });

  it('treats future timestamps as just now and unsafe values as em dash', () => {
    expectRelativeCases([
      { secs: String(Math.floor(NOW / 1000) + 60), expected: 'just now' }, { secs: null, expected: '\u2014' },
      { secs: undefined, expected: '\u2014' }, { secs: '9007199254740993', expected: '\u2014' }, { secs: 'not-a-number', expected: '\u2014' },
    ]);
  });
});

describe('normalizeDisplayPath and middleTruncatePath', () => {
  it('preserves UNC and extended-length prefixes while collapsing separators elsewhere', () => {
    const paths: PathCase[] = [
      { input: '\\\\server\\share\\a\\\\b\\c', expected: '\\\\server\\share\\a\\b\\c' },
      { input: '\\\\?\\C:\\temp\\\\logs\\app.log', expected: '\\\\?\\C:\\temp\\logs\\app.log' },
      { input: '\\\\?\\UNC\\server\\share\\\\dir\\file.txt', expected: '\\\\?\\UNC\\server\\share\\dir\\file.txt' },
    ];
    expectNormalizeCases(paths);
    expectTruncateCases(paths.slice(0, 2));
  });

  it('preserves leading \\\\ for long UNC paths', () => {
    const input = '\\\\server\\share\\very\\long\\folder\\structure\\that\\keeps\\going\\cache\\file.log';
    const out = middleTruncatePath({ path: input, maxLen: 36 });
    expect(out.startsWith('\\\\server\\share\\')).toBe(true);
    expect(out.includes('\\server\\...')).toBe(false);
  });
});

describe('middleTruncatePath (grapheme-aware)', () => {
  const ELLIPSIS = '\u2026';
  const BROOM = '\u{1F9F9}'; // one grapheme, one surrogate pair
  const FAMILY = '\u{1F468}\u200D\u{1F469}\u200D\u{1F467}\u200D\u{1F466}'; // one ZWJ grapheme

  function hasLoneSurrogate(value: string): boolean {
    for (let i = 0; i < value.length; i += 1) {
      const code = value.charCodeAt(i);
      if (code >= 0xd800 && code <= 0xdbff) {
        const next = value.charCodeAt(i + 1);
        if (!(next >= 0xdc00 && next <= 0xdfff)) return true;
        i += 1;
      } else if (code >= 0xdc00 && code <= 0xdfff) {
        return true;
      }
    }
    return false;
  }

  function graphemeLength(value: string): number {
    const Segmenter = (Intl as unknown as {
      Segmenter?: new (locales?: string, options?: { granularity?: 'grapheme' }) => {
        segment(input: string): Iterable<unknown>;
      };
    }).Segmenter;
    if (!Segmenter) return Array.from(value).length;
    return Array.from(new Segmenter(undefined, { granularity: 'grapheme' }).segment(value)).length;
  }

  it('preserves ASCII truncation behavior', () => {
    const path = 'C:\\Users\\Alice\\AppData\\Local\\Temp\\deep\\nested\\cache-file-with-a-long-name.tmp';
    const out = middleTruncatePath({ path, maxLen: 40 });
    expect(out.startsWith('C:\\')).toBe(true);
    expect(out.endsWith('.tmp')).toBe(true);
    expect(out).toContain(ELLIPSIS);
    expect(out.length).toBeLessThanOrEqual(40);
  });

  it('counts a single emoji filename as one grapheme at the limit', () => {
    const path = `C:\\Temp\\cache\\${BROOM}`;
    const out = middleTruncatePath({ path, maxLen: 15 });
    expect(out).toBe(path);
    expect(out).toContain(BROOM);
    expect(hasLoneSurrogate(out)).toBe(false);
  });

  it('does not split an emoji surrogate pair when truncating a leaf', () => {
    const path = `C:\\Temp\\cache\\${BROOM.repeat(24)}report.bin`;
    const out = middleTruncatePath({ path, maxLen: 22 });
    expect(out).toContain(ELLIPSIS);
    expect(hasLoneSurrogate(out)).toBe(false);
    expect(out.split(BROOM).length - 1).toBeGreaterThan(0);
    expect(Array.from(out).length).toBeLessThanOrEqual(22);
  });

  it('keeps multi-code-point emoji clusters intact', () => {
    const path = `C:\\Temp\\cache\\${FAMILY.repeat(20)}archive.zip`;
    const out = middleTruncatePath({ path, maxLen: 24 });
    expect(hasLoneSurrogate(out)).toBe(false);
    expect(out.split(FAMILY).length - 1).toBeGreaterThan(0);
    // A whole family has three ZWJ joiners; a cut cluster would leave 1 or 2.
    expect((out.split('\u200D').length - 1) % 3).toBe(0);
  });

  it('preserves a Unicode Prepend grapheme at the extension boundary', () => {
    const PREPEND = '\u0600'; // Arabic number sign: Prepend property, clusters with the following '.'
    const path = `C:\\Temp\\cache\\${'a'.repeat(30)}${PREPEND}.txt`;
    const out = middleTruncatePath({ path, maxLen: 22 });
    expect(out).toContain(ELLIPSIS);
    // U+0600 and the dot form one grapheme; it must survive intact, not be dropped
    // or split when the tail is selected across the extension boundary.
    expect(out).toContain(`${PREPEND}.`);
    expect(out.endsWith('.txt')).toBe(true);
    expect(graphemeLength(out)).toBeLessThanOrEqual(22);
  });

  it('preserves a Unicode Prepend grapheme at the head boundary with a short core', () => {
    const PREPEND = '\u0600'; // clusters with the following '.' — a Prepend character
    const path = `C:\\Temp\\cache\\aa${PREPEND}.123456789`;
    const maxLen = 13;
    const out = middleTruncatePath({ path, maxLen });
    expect(out).toContain(ELLIPSIS);
    // The head must be taken from the original leaf so the complete U+0600 +
    // '.' grapheme survives; a count-derived core loses the dot and would leave
    // U+0600 clustered with the ellipsis instead.
    expect(out).toContain(`${PREPEND}.`);
    expect(out).not.toContain(`${PREPEND}${ELLIPSIS}`);
    expect(graphemeLength(out)).toBeLessThanOrEqual(maxLen);
  });
});
