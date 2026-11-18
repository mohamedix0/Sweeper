import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  COMPACT_SELECTION_THRESHOLD,
  EXPLANATION_FULL_LIMIT,
  EXPLANATION_SUMMARY_LIMIT,
  buildSelectionIndex,
  expandPreviewSelectionIds,
  planExplanationTasks,
  preparePreviewSelection,
  selectedCategoryCountFromIndex,
  selectedBytesFromIndex,
} from './components/scan/previewPerf';

function largeSelection(n: number, prefix = 'user-temp'): Set<string> {
  const set = new Set<string>();
  for (let i = 1; i <= n; i += 1) set.add(`${prefix}::${i}`);
  return set;
}

describe('T3 compact preview selection', () => {
  it('keeps small selections explicit so per-item preview semantics are unchanged', async () => {
    const ids = ['i1', 'i2', 'custom-path-1::7'];
    const result = await preparePreviewSelection(new Set(ids));
    expect(result).toEqual(ids);
  });

  it('reduces a 20k selection to a compact descriptor that expands back exactly', async () => {
    const selected = largeSelection(20_000);
    const wire = await preparePreviewSelection(selected);
    expect(wire.length).toBeLessThanOrEqual(8);
    expect(wire.length).toBeLessThan(COMPACT_SELECTION_THRESHOLD);

    const expanded = expandPreviewSelectionIds(wire);
    expect(expanded.length).toBe(selected.size);
    expect(new Set(expanded)).toEqual(selected);
  });

  it('preserves individually deselected items exactly in compact mode', async () => {
    const selected = largeSelection(500);
    selected.delete('user-temp::250');
    selected.delete('user-temp::251');
    selected.add('recycle-bin'); // non-range id must survive verbatim

    const wire = await preparePreviewSelection(selected);
    const expanded = expandPreviewSelectionIds(wire);
    expect(new Set(expanded)).toEqual(selected);
    expect(expanded).not.toContain('user-temp::250');
    expect(expanded).not.toContain('user-temp::251');
    expect(expanded).toContain('recycle-bin');
  });

  it('reports determinate preparation progress and yields between chunks', async () => {
    const selected = largeSelection(4_500);
    const seen: Array<[number, number]> = [];
    await preparePreviewSelection(selected, (completed, total) => seen.push([completed, total]));
    expect(totalSeen(seen, selected.size)).toBe(true);
    expect(seen.length).toBeGreaterThan(1);
  });

  function totalSeen(samples: Array<[number, number]>, total: number): boolean {
    return samples.some(([done, t]) => t === total && done === total);
  }

  it('never builds a giant NUL-joined selection key in the view source', () => {
    const source = readFileSync(resolve(__dirname, 'components/ScanAndPreviewView.tsx'), 'utf8');
    expect(source).not.toMatch(/join\(['"]\\u0000['"]\)/);
    expect(source).not.toMatch(/join\(['"]\u0000['"]\)/);
  });
});

describe('T3 bounded explanation resolution', () => {
  it('plans one task per selected item for small selections', () => {
    const selected = new Set(['i1', 'i2']);
    const items = new Map([
      ['i1', { targetId: 't', targetRisk: 'Safe' as const, path: 'item-1' }],
      ['i2', { targetId: 't', targetRisk: 'Safe' as const, path: 'item-2' }],
    ]);
    expect(planExplanationTasks({ selected, targetIdFor: (id) => items.get(id)?.targetId }).map((t) => t.itemId).sort()).toEqual(['i1', 'i2']);
    expect(EXPLANATION_FULL_LIMIT).toBeGreaterThanOrEqual(2);
  });

  it('bounds a 20k selection to a per-target summary set', () => {
    const selected = largeSelection(20_000);
    const items = new Map<string, { targetId: string; targetRisk: 'Safe' | 'Caution' | 'Unknown'; path: string }>();
    for (const id of selected) items.set(id, { targetId: 'user-temp', targetRisk: 'Safe', path: id });

    const tasks = planExplanationTasks({ selected, targetIdFor: (id) => items.get(id)?.targetId });
    expect(tasks.length).toBeLessThanOrEqual(EXPLANATION_SUMMARY_LIMIT);
    expect(tasks.length).toBeLessThan(selected.size);
    expect(new Set(tasks.map((t) => t.targetId)).size).toBe(tasks.length);
  });
});

describe('T3 memoised derived selection figures', () => {
  it('computes selected bytes and category count from a Map index without rescanning targets', () => {
    const targets = [
      { targetId: 'a', risk: 'Safe' as const, items: [
        { id: 'a1', size: '100' }, { id: 'a2', size: '250' },
      ] },
      { targetId: 'b', risk: 'Caution' as const, items: [
        { id: 'b1', size: '1' },
      ] },
    ] as any;
    const index = buildSelectionIndex({ targets });
    expect(index.size).toBe(3);
    expect(selectedBytesFromIndex({ index, selected: new Set(['a1', 'a2']) })).toBe('350');
    expect(selectedCategoryCountFromIndex({ index, selected: new Set(['a1', 'b1']) })).toBe(2);
    expect(selectedCategoryCountFromIndex({ index, selected: new Set(['a1', 'a2']) })).toBe(1);
  });
});
