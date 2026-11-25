import { describe, expect, it } from 'vitest';
import type { TargetScanResultDto } from '../../contract/bindings';
import { createMockController } from './controller';

type Client = ReturnType<typeof createMockController>;
type Rejection = { code: string; details: Record<string, unknown> };

async function rejectionOf<T>(promise: Promise<T>): Promise<Rejection> {
  try {
    await promise;
  } catch (error) {
    return error as Rejection;
  }
  throw new Error('Expected the mock call to reject.');
}

function eligibleIds(targets: TargetScanResultDto[]): string[] {
  return targets
    .filter((target) => target.risk === 'Safe')
    .flatMap((target) => target.items.filter((item) => item.ageEligible).map((item) => item.id));
}

const SCAN_RUNNING_REJECTION = {
  code: 'internal',
  details: { reason: 'Clean rejected: a Scan is already running' },
};

/** Start a scan and leave its worker in flight via the test-only progress delay. */
function startInFlightScan(client: Client): { scan: Promise<{ sessionId: string }>; sessionId: () => string } {
  let sessionId = '';
  const scan = client.startScan(null, (event) => {
    if (event.kind === 'started') sessionId = event.session_id;
  });
  return { scan, sessionId: () => sessionId };
}

async function completedSelection(client: Client): Promise<{ sessionId: string; ids: string[] }> {
  const { sessionId } = await client.startScan(null);
  const results = await client.getScanResults(sessionId);
  const ids = eligibleIds(results.targets);
  expect(ids.length).toBeGreaterThan(0);
  return { sessionId, ids };
}

describe('mock Clean-while-Scan gate (#143)', () => {
  it('rejects Preview token issuance during an in-flight Scan and issues no token', async () => {
    const client = createMockController();
    client.setScanStepDelayMs(25);
    const { scan, sessionId } = startInFlightScan(client);

    const results = await client.getScanResults(sessionId());
    const ids = eligibleIds(results.targets);
    expect(ids.length).toBeGreaterThan(0);

    expect(await rejectionOf(client.previewClean(sessionId(), ids, null))).toEqual(SCAN_RUNNING_REJECTION);

    // The rejected call consumed no token: the first token is issued only now.
    await scan;
    const after = await client.previewClean(sessionId(), ids, null);
    expect(after.previewToken).toBe('mock-preview-1');
  });

  it('rejects Clean execution during an in-flight Scan without consuming the token or deleting', async () => {
    const client = createMockController();
    const { sessionId, ids } = await completedSelection(client);
    const preview = await client.previewClean(sessionId, ids, null);

    client.setScanStepDelayMs(25);
    const { scan } = startInFlightScan(client);

    expect(await rejectionOf(client.executeClean(preview.previewToken))).toEqual(SCAN_RUNNING_REJECTION);

    // The rejection consumed neither the token nor any items: it still succeeds
    // unchanged once the conflicting Scan has finished.
    await scan;
    const summary = await client.executeClean(preview.previewToken);
    expect(summary.aborted).toBe(false);
    expect(Number(summary.deletedCount)).toBe(ids.length);
  });
});
