import { describe, expect, it } from 'vitest';
import { createMockController } from '../mock';

type Client = ReturnType<typeof createMockController>;
type Selection = { client: Client; sessionId: string; ids: string[] };

async function selectEligible(client: Client): Promise<{ sessionId: string; ids: string[] }> {
  const { sessionId } = await client.startScan(null);
  const results = await client.getScanResults(sessionId);
  const ids = results.targets
    .filter((target) => target.risk === 'Safe')
    .flatMap((target) => target.items.filter((item) => item.ageEligible).map((item) => item.id));
  return { sessionId, ids };
}

/** Eligible Safe items are selectable without typed confirmation or advanced settings. */
async function startAndSelect(): Promise<Selection> {
  const client = createMockController();
  return { client, ...(await selectEligible(client)) };
}

describe('mock Scan -> Preview -> Clean re-validation fidelity', () => {
  it('Preview reports Skipped metadata and drops invalidated items from the selectable set', async () => {
    const { client, sessionId, ids } = await startAndSelect();
    expect(ids.length).toBeGreaterThanOrEqual(3);

    client.invalidateItem(ids[0], { reason: 'File locked' });
    const preview = await client.previewClean(sessionId, ids, null);

    expect(preview.skippedCount).toBe('1');
    expect(preview.skippedReasons).toEqual(['File locked']);
    expect(preview.totalSelectedCount).toBe(String(ids.length - 1));
    expect(preview.breakdown.reduce((sum, row) => sum + Number(row.selectedCount), 0)).toBe(ids.length - 1);
  });

  it('Clean honours invalidation after Preview and emits a typed Skipped reason', async () => {
    const { client, sessionId, ids } = await startAndSelect();
    const preview = await client.previewClean(sessionId, ids, null);

    client.invalidateItem(ids[0], { reason: 'Failed re-validation' });
    const summary = await client.executeClean(preview.previewToken);

    expect(summary.aborted).toBe(false);
    expect(summary.deletedCount).toBe(String(ids.length - 1));
    expect(summary.skippedCount).toBe('1');
    expect(summary.skippedReasons).toEqual([{ reason: 'Skipped: Failed re-validation', count: '1' }]);
  });

  it('Clean honours an abort invalidation and emits a typed abort reason without deleting', async () => {
    const { client, sessionId, ids } = await startAndSelect();
    const preview = await client.previewClean(sessionId, ids, null);

    client.invalidateItem(ids[0], { reason: 'Circuit breaker exceeded', abort: true });
    const summary = await client.executeClean(preview.previewToken);

    expect(summary.aborted).toBe(true);
    expect(summary.abortReason).toBe('aborted: Circuit breaker exceeded');
    expect(summary.deletedCount).toBe('0');
    expect(summary.bytesFreed).toBe('0');
    expect(summary.skippedReasons).toEqual([{ reason: 'Skipped: Circuit breaker exceeded', count: '1' }]);
  });

  it('reset clears re-validation state', async () => {
    const { client, ids } = await startAndSelect();
    client.invalidateItem(ids[0], { reason: 'File locked' });

    client.reset();
    const { sessionId, ids: freshIds } = await selectEligible(client);
    const preview = await client.previewClean(sessionId, freshIds, null);

    expect(preview.skippedCount).toBeUndefined();
  });
});
