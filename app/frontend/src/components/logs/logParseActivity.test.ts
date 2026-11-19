import { describe, expect, it } from 'vitest';
import { deriveActivitySummary } from './logParseActivity';
import { parseAuditRows } from './logParseRows';
import { summarizeLog } from './logParseSummary';
import type { LogMeta } from './logParseTypes';

function lines(...rows: object[]): string {
  return rows.map((row) => JSON.stringify(row)).join('\n');
}

const meta: LogMeta = { id: 's1', name: 'scan-1.jsonl', sizeBytes: '10', modifiedUnixSecs: '1710000000' };

describe('Activity summary uses the canonical pluralizer', () => {
  it('reads singular file/category nouns for a single-item Scan', () => {
    const summary = deriveActivitySummary(lines({
      record: 'outcome', event: 'scan', status: 'completed',
      categories_count: 1, total_eligible_files: 1,
    }));
    expect(summary.eligible).toBe('1 file · 1 category');
    expect(summary.skipped).toBe('0 files · 0 categories');
  });

  it('reads plural nouns for a multi-item Scan', () => {
    const summary = deriveActivitySummary(lines({
      record: 'outcome', event: 'scan', status: 'completed',
      categories_count: 2, total_eligible_files: 3, total_skipped_files: 2,
    }));
    expect(summary.eligible).toBe('3 files · 2 categories');
    expect(summary.skipped).toBe('2 files · 0 categories');
  });

  it('reads singular and plural nouns for Clean outcomes', () => {
    const single = deriveActivitySummary(lines(
      { record: 'intent', session_id: 'one' },
      { record: 'outcome', target_id: 'temp-a', status: 'deleted', bytes: 1024 },
    ));
    expect(single.eligible).toBe('1 file · 1 category');
    expect(single.skipped).toBe('0 files · 0 categories');

    const many = deriveActivitySummary(lines(
      { record: 'intent', session_id: 'many' },
      { record: 'outcome', target_id: 'temp-a', status: 'deleted', bytes: 1024 },
      { record: 'outcome', target_id: 'temp-b', status: 'deleted', bytes: 2048 },
      { record: 'outcome', target_id: 'temp-a', status: 'skipped', bytes: 512 },
    ));
    expect(many.eligible).toBe('2 files · 2 categories');
    expect(many.skipped).toBe('1 file · 1 category');
  });
});

describe('Activity row and status labels use the canonical pluralizer', () => {
  it('shows "1 category" instead of "1 categories" in the System Scan row', () => {
    const content = lines({ record: 'outcome', event: 'scan', status: 'completed', categories_count: 1 });
    const row = parseAuditRows(content).find((r) => r.category === 'System Scan');
    expect(row?.location).toBe('1 category');
  });

  it('shows the plural label for multiple categories in the System Scan row', () => {
    const content = lines({ record: 'outcome', event: 'scan', status: 'completed', categories_count: 2 });
    const row = parseAuditRows(content).find((r) => r.category === 'System Scan');
    expect(row?.location).toBe('2 categories');
  });

  it('uses the singular/plural category noun in the Scan log title', () => {
    const single = summarizeLog(meta, lines({ record: 'outcome', event: 'scan', status: 'completed', categories_count: 1 }));
    expect(single.title).toContain('1 category');
    expect(single.title).not.toContain('1 categories');

    const many = summarizeLog(meta, lines({ record: 'outcome', event: 'scan', status: 'completed', categories_count: 2 }));
    expect(many.title).toContain('2 categories');
  });
});
