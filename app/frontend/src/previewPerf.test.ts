import { describe, expect, it } from 'vitest';
import {
  COMPACT_SELECTION_THRESHOLD,
  EXPLANATION_FULL_LIMIT,
  EXPLANATION_SUMMARY_LIMIT,
  buildKnownRiskSources,
  buildSelectionIndex,
  expandPreviewSelectionIds,
  planExplanationTasks,
  mergeRiskSources,
  rangeBounds,
  riskCoverageComplete,
  preparePreviewSelection,
  mapWithConcurrency,
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
    expect(wire).toHaveLength(1); // one compact descriptor token, not a giant joined key
    expect(wire[0]!.length).toBeLessThan(1024);

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
    let timerRan = false;
    setTimeout(() => { timerRan = true; }, 0);
    await preparePreviewSelection(selected, (completed, total) => {
      // A timer queued before preparation must run before a later chunk reports
      // progress; otherwise the "yield" claims are not observable.
      if (seen.length >= 1) expect(timerRan).toBe(true);
      seen.push([completed, total]);
    });
    expect(totalSeen(seen, selected.size)).toBe(true);
    expect(seen.length).toBeGreaterThan(1);
    expect(timerRan).toBe(true);
  });

  it('keeps noncanonical numeric item IDs verbatim through the compact round-trip', async () => {
    const selected = largeSelection(200);
    selected.delete('user-temp::7');
    selected.add('user-temp::007'); // leading zeroes must not be coerced to 7
    const wire = await preparePreviewSelection(selected);
    const expanded = expandPreviewSelectionIds(wire);
    expect(new Set(expanded)).toEqual(selected);
    expect(expanded).toContain('user-temp::007');
    expect(expanded).not.toContain('user-temp::7');
  });

  it('rejects nonnumeric and boolean range bounds instead of coercing them', () => {
    const descriptor = JSON.stringify({ ranges: [{ prefix: 'user-temp', start: '1', end: 200 }], explicit: [] });
    const wire = [`SWSELECT1:${descriptor}`];
    expect(expandPreviewSelectionIds(wire)).toEqual(wire);

    const booleanBound = JSON.stringify({ ranges: [{ prefix: 'user-temp', start: true, end: 200 }], explicit: [] });
    const booleanWire = [`SWSELECT1:${booleanBound}`];
    expect(expandPreviewSelectionIds(booleanWire)).toEqual(booleanWire);
  });

  it('rejects an unsafe end bound at the range-validation boundary', () => {
    // Assert at rangeBounds, before expandRanges can iterate. Going through
    // expandPreviewSelectionIds here would hang under mutation: end = 2**53 is
    // unsafe, and `n += 1` cannot advance past 2**53, so the loop never ends.
    // A direct validation assertion fails cleanly if the end-bound guard is
    // removed while keeping the unsafe end-bound fixture.
    const unsafeEnd = Number.MAX_SAFE_INTEGER + 1;
    expect(rangeBounds({ prefix: 'user-temp', start: Number.MAX_SAFE_INTEGER, end: unsafeEnd })).toBeNull();
  });

  it('expands a valid range ending at Number.MAX_SAFE_INTEGER', () => {
    const max = Number.MAX_SAFE_INTEGER;
    const descriptor = JSON.stringify({ ranges: [{ prefix: 'user-temp', start: max - 1, end: max }], explicit: [] });
    const wire = [`SWSELECT1:${descriptor}`];
    expect(expandPreviewSelectionIds(wire)).toEqual([`user-temp::${max - 1}`, `user-temp::${max}`]);
  });

  function totalSeen(samples: Array<[number, number]>, total: number): boolean {
    return samples.some(([done, t]) => t === total && done === total);
  }
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
    expect(tasks).toHaveLength(1);
    expect(tasks[0].targetId).toBe('user-temp');
    expect(selected.has(tasks[0].itemId)).toBe(true);
  });

  it('shows a warning for every selected elevated-risk item even when the target cap is reached', async () => {
    const { deriveCalloutItemsFromSelection } = await import('./components/PreviewRiskCallouts');
    const targets: any[] = [];
    const selected = new Set<string>();
    const itemContext = new Map<string, any>();
    for (let t = 0; t < 20; t += 1) {
      const targetId = t === 19 ? 'custom-path-1' : `caution-target-${t}`;
      const risk = t === 19 ? 'Unknown' : 'Caution';
      const items = Array.from({ length: 5 }, (_, i) => ({
        id: `${targetId}::${i + 1}`,
        path: `C:\\Users\\Alice\\Temp\\${targetId}-${i + 1}`,
        size: '100',
        ageEligible: true,
      }));
      for (const item of items) {
        selected.add(item.id);
        itemContext.set(item.id, { target: { targetId, risk }, item });
      }
      targets.push({ targetId, risk, items });
    }

    const tasks = planExplanationTasks({ selected, targetIdFor: (id) => itemContext.get(id)?.target.targetId });
    expect(tasks.length).toBeLessThanOrEqual(EXPLANATION_SUMMARY_LIMIT); // IPC stays bounded

    const known = buildKnownRiskSources({ selection: selected, itemContext });
    const callouts = deriveCalloutItemsFromSelection({
      selectedIds: selected,
      sources: known,
      isCustomPath: (targetId) => targetId.startsWith('custom-path'),
    });
    expect(callouts).toHaveLength(selected.size);
    expect(new Set(callouts.map((c) => c.id))).toEqual(selected);
    expect(callouts.filter((c) => c.risk === 'Custom Path')).toHaveLength(5);
    expect(riskCoverageComplete({ selection: selected, sources: known })).toBe(true);
  });
});

describe('T3 stale explanation-pool cancellation', () => {
  it('does not start queued IPC work once the current Preview changes', async () => {
    const started: number[] = [];
    const skipped: number[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    let cancelled = false;

    // Concurrency 2 keeps two calls in flight while two stay queued. Every
    // worker awaits the same gate, so a runner cannot churn through queued work
    // before cancellation is observed; this proves the shouldStop check runs
    // before each queued task, for every runner in the pool. Both in-flight
    // workers observe cancellation, so onSkip must be claimed once per queued
    // item even though two runners reach the skip path.
    const run = mapWithConcurrency(
      [0, 1, 2, 3],
      2,
      async (n) => {
        started.push(n);
        await gate;
        return n;
      },
      { shouldStop: () => cancelled, onSkip: (n) => { skipped.push(n); return -1; } },
    );
    expect(started).toEqual([0, 1]); // only in-flight calls started
    cancelled = true;
    release();
    expect(await run).toEqual([0, 1, -1, -1]); // in-flight settle, queued are skipped
    expect(started).toEqual([0, 1]); // no queued call started after shouldStop flipped
    expect(skipped).toEqual([2, 3]); // one onSkip per queued item, no duplicates
  });
});

describe('T3 known-risk source merging', () => {
  it('never downgrades a known elevated Risk when an explanation reports Safe', async () => {
    const { deriveCalloutItemsFromSelection } = await import('./components/PreviewRiskCallouts');
    const known = [{
      itemId: 'caution::1', title: 'C:\\Cache\\entry', description: 'known', risk: 'Caution' as const, targetId: 'caution',
    }];
    const resolved = [{
      itemId: 'caution::1', title: 'Cache entry', description: 'KB', risk: 'Safe' as const, targetId: 'caution',
    }];
    const sources = mergeRiskSources(known, resolved);
    const callouts = deriveCalloutItemsFromSelection({ selectedIds: new Set(['caution::1']), sources });
    expect(callouts).toHaveLength(1);
    expect(callouts[0].risk).toBe('Caution');
    expect(callouts[0].identity).toBe('C:\\Cache\\entry');
  });

  it('keeps each selected file path as its warning identity within a shared Target', async () => {
    const { deriveCalloutItemsFromSelection } = await import('./components/PreviewRiskCallouts');
    const known = [
      { itemId: 'caution::1', title: 'C:\\Cache\\first.tmp', description: 'known', risk: 'Caution' as const, targetId: 'caution' },
      { itemId: 'caution::2', title: 'C:\\Cache\\second.tmp', description: 'known', risk: 'Caution' as const, targetId: 'caution' },
    ];
    const resolved = [
      { itemId: 'caution::1', title: 'Shared Target Name', description: 'KB first', risk: 'Caution' as const, targetId: 'caution' },
      { itemId: 'caution::2', title: 'Shared Target Name', description: 'KB second', risk: 'Caution' as const, targetId: 'caution' },
    ];
    const callouts = deriveCalloutItemsFromSelection({
      selectedIds: new Set(['caution::1', 'caution::2']),
      sources: mergeRiskSources(known, resolved),
    });
    expect(callouts.map((c) => c.identity)).toEqual(['C:\\Cache\\first.tmp', 'C:\\Cache\\second.tmp']);
    expect(callouts.map((c) => c.explanation)).toEqual(['KB first', 'KB second']);
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
