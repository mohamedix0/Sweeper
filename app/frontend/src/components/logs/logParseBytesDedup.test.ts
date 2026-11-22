import { describe, expect, it } from 'vitest';
import { deriveActivitySummary } from './logParseActivity';
import { summarizeLog } from './logParseSummary';
import type { LogMeta } from './logParseTypes';

function lines(...rows: object[]): string {
  return rows.map((row) => JSON.stringify(row)).join('\n');
}

const meta: LogMeta = { id: 's1', name: 'clean-1.jsonl', sizeBytes: '10', modifiedUnixSecs: '1710000000' };

// Legacy/old-shape log: a Tier 3 / WholeCache root writes one `deleted` outcome
// per child plus a root outcome whose `bytes` repeats the sum of the children
// (issue #135). Naively summing every outcome reports 2 KB instead of 1 KB.
const doubleCountedLog = lines(
  { record: 'intent', target_id: 'wc-1', path: 'C:\\cache', size: 1024 },
  { record: 'outcome', target_id: 'wc-1', path: 'C:\\cache\\a.bin', bytes: 512, status: 'deleted' },
  { record: 'outcome', target_id: 'wc-1', path: 'C:\\cache\\b.bin', bytes: 512, status: 'deleted' },
  { record: 'outcome', target_id: 'wc-1', path: 'C:\\cache', bytes: 1024, status: 'deleted' },
);

describe('issue #135: log byte totals are not double-counted', () => {
  it('Activity History sums child bytes without the repeating root outcome', () => {
    expect(deriveActivitySummary(doubleCountedLog).primaryValue).toBe('1 KB');
  });

  it('the summary index titles use child bytes without the repeating root outcome', () => {
    const summary = summarizeLog(meta, doubleCountedLog);
    expect(summary.title).toContain('1 KB');
    expect(summary.title).not.toContain('2 KB');
  });

  it('falls back to size when bytes is absent', () => {
    const sizeOnly = lines(
      { record: 'outcome', target_id: 'wc-2', path: 'C:\\cache\\size-a.bin', size: 512, status: 'deleted' },
      { record: 'outcome', target_id: 'wc-2', path: 'C:\\cache\\size-b.bin', size: 512, status: 'deleted' },
      { record: 'outcome', target_id: 'wc-2', path: 'C:\\cache', size: 1024, status: 'deleted' },
    );
    expect(deriveActivitySummary(sizeOnly).primaryValue).toBe('1 KB');
  });

  it('classifies a large deleted-outcome set without an O(n^2) scan', () => {
    const rows: object[] = [];
    for (let i = 0; i < 10_240; i += 1) {
      rows.push({ record: 'outcome', target_id: 'wc-3', path: `C:\\cache\\f${i}.bin`, bytes: 1, status: 'deleted' });
    }
    rows.push({ record: 'outcome', target_id: 'wc-3', path: 'C:\\cache', bytes: 10_240, status: 'deleted' });
    const started = Date.now();
    const summary = summarizeLog(meta, lines(...rows));
    expect(Date.now() - started).toBeLessThan(5000);
    expect(summary.title).toContain('10 KB');
    expect(summary.title).not.toContain('20 KB');
  });
});
