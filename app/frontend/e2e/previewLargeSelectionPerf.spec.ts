import { expect, test, type Page } from '@playwright/test';

/**
 * R4 regression (#61): generating a Preview for a very large selection must
 * keep the UI responsive. All data is synthetic (Alice-style fixture IDs, no
 * real paths); the mock backend is instrumented in the browser only, and the
 * Edge channel comes from playwright.config.ts. No real Clean is performed.
 */
const ITEM_COUNT = 20_000;
const EXPLAIN_CALL_BOUND = 32;
const PREVIEW_ID_BOUND = 8;
// Shared CI runners add scheduling jitter; 60 ms still catches the 90 ms
// final-render stall this regression guards against.
const MAX_MAIN_THREAD_GAP_MS = 60;
const CTA_BOUND_MS = 15_000;

type Metrics = {
  explainCalls: number;
  previewIds: number;
  payloadIdsMatch: boolean;
  maxMainThreadGapMs: number;
  determinateProgressSeen: boolean;
};

async function installGapSampler(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const g = globalThis as unknown as {
      __gapSamples: number[];
      __lastFrame: number;
      __resetGaps: () => void;
      __perf: Record<string, number | boolean>;
    };
    g.__gapSamples = [];
    g.__lastFrame = 0;
    g.__perf = {};
    g.__resetGaps = () => {
      g.__gapSamples = [];
      // 0 means "no baseline yet": the first frame after a reset spans the
      // click/effect scheduling, not a renderer stall, so it is not sampled.
      g.__lastFrame = 0;
    };
    // Record a determinate progress meter *while preparation is running*, on
    // the same animation frame as the gap sampler. Checking for the wrapper at
    // the Preview IPC call is not evidence that the meter ever showed a real
    // value (and progress is cleared before that call once encoding finishes).
    const tick = (now: number) => {
      if (g.__lastFrame > 0) g.__gapSamples.push(now - g.__lastFrame);
      g.__lastFrame = now;
      const meter = document.querySelector('.preview-preparing progress');
      if (meter) {
        const value = Number((meter as HTMLProgressElement).value);
        const max = Number((meter as HTMLProgressElement).max);
        if (max > 0 && value > 0 && value < max) g.__perf.determinateProgressSeen = true;
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

async function installLargeSelectionMock(page: Page, count: number): Promise<void> {
  await page.waitForFunction(() => {
    const mock = (window as unknown as { __sweeperMock?: { startScan?: unknown } }).__sweeperMock;
    return typeof mock?.startScan === 'function';
  });

  await page.evaluate((n) => {
    const mock = (window as unknown as { __sweeperMock: Record<string, any> }).__sweeperMock;
    const metrics = (window as unknown as { __perf: Record<string, number | boolean> }).__perf;
    metrics.explainCalls = 0;
    metrics.previewIds = 0;
    metrics.previewCalls = 0;
    metrics.determinateProgressSeen = false;

    const items = Array.from({ length: n }, (_, idx) => ({
      id: `user-temp::${idx + 1}`,
      targetId: 'user-temp',
      path: `C:\\Users\\Alice\\AppData\\Local\\Temp\\item-${idx + 1}`,
      size: '100',
      mtimeUnixSecs: '1700000000',
      ageEligible: true,
    }));
    const bytes = String(n * 100);
    const target = {
      targetId: 'user-temp', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe',
      resolvedPaths: ['C:\\Users\\Alice\\AppData\\Local\\Temp'], scanned: true, statusReason: null,
      totalBytes: bytes, eligibleBytes: bytes, fileCount: String(n), eligibleCount: String(n),
      skippedCount: '0', items,
    };
    const fixtureIds = new Set(items.map((item) => item.id));

    // Mirror of the compact descriptor decoder: the test must verify the
    // payload contains exactly the fixture selection, not just that it is short.
    function decode(ids: string[]): string[] {
      if (ids.length === 1 && ids[0].startsWith('SWSELECT1:')) {
        const descriptor = JSON.parse(ids[0].slice('SWSELECT1:'.length)) as {
          ranges: Array<{ prefix: string; start: number; end: number }>;
          explicit: string[];
        };
        const expanded: string[] = [];
        for (const range of descriptor.ranges) {
          for (let seq = range.start; seq <= range.end; seq += 1) expanded.push(`${range.prefix}::${seq}`);
        }
        return expanded.concat(descriptor.explicit);
      }
      return ids;
    }

    mock.startScan = async () => ({ sessionId: 'perf-scan' });
    mock.getScanResults = async () => ({
      sessionId: 'perf-scan', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [target],
    });
    mock.explainItem = async (itemId: string) => {
      metrics.explainCalls = Number(metrics.explainCalls) + 1;
      return { itemId, title: itemId, description: 'Synthetic Knowledge Base entry', risk: 'Safe' };
    };
    mock.previewClean = async (_sessionId: string, itemIds: string[]) => {
      metrics.previewCalls = Number(metrics.previewCalls) + 1;
      metrics.previewIds = itemIds.length;
      const actualIds = decode(itemIds);
      metrics.payloadIdsMatch = actualIds.length === fixtureIds.size && actualIds.every((id) => fixtureIds.has(id));
      const actualBytes = String(actualIds.length * 100);
      return {
        previewToken: 'perf-token',
        breakdown: [{ targetId: 'user-temp', targetName: 'User Temp', selectedCount: String(actualIds.length), selectedBytes: actualBytes }],
        totalSelectedCount: String(actualIds.length),
        totalSelectedBytes: actualBytes,
      };
    };
  }, count);
}

async function readMetrics(page: Page): Promise<Metrics> {
  return page.evaluate(() => {
    const g = globalThis as unknown as { __gapSamples: number[] };
    const p = (window as unknown as { __perf: Record<string, number | boolean> }).__perf;
    const gaps = g.__gapSamples.slice().sort((a, b) => a - b);
    if (gaps.length === 0) throw new Error('animation-frame sampler recorded no frames');
    return {
      explainCalls: Number(p.explainCalls),
      previewIds: Number(p.previewIds),
      payloadIdsMatch: Boolean(p.payloadIdsMatch),
      maxMainThreadGapMs: gaps[gaps.length - 1],
      determinateProgressSeen: Boolean(p.determinateProgressSeen),
    };
  });
}

test.describe('Sweeper Preview responsiveness at large selections (mock sandbox)', () => {
  test('20k selection: bounded IPC, compact selection, determinate progress, responsive UI', async ({ page }) => {
    await installGapSampler(page);
    await page.goto('/');
    await installLargeSelectionMock(page, ITEM_COUNT);

    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });

    // Discard scan-time frames and any focus explanation so only Preview is measured.
    await page.evaluate(() => {
      const p = (window as unknown as { __perf: Record<string, number | boolean> }).__perf;
      p.explainCalls = 0;
      (globalThis as unknown as { __resetGaps: () => void }).__resetGaps();
    });

    const review = page.getByRole('button', { name: /Review & Clean/ });
    await expect(review).toBeEnabled();
    const started = Date.now();
    await review.click();

    const cta = page.getByRole('button', { name: /Delete .* permanently/ });
    const framesBeforeEnable = await page.evaluate(() => (globalThis as unknown as { __gapSamples: number[] }).__gapSamples.length);
    await expect(cta).toBeEnabled({ timeout: CTA_BOUND_MS });
    // The final render that enables the CTA can itself occupy a frame interval;
    // wait for the sampler to record a frame after enable before measuring.
    await page.waitForFunction(
      (before) => (globalThis as unknown as { __gapSamples: number[] }).__gapSamples.length > before,
      framesBeforeEnable,
    );
    const elapsedMs = Date.now() - started;

    const metrics = await readMetrics(page);
    console.log('T3 preview perf @20k', { ...metrics, elapsedMs });
    expect(metrics.explainCalls, 'explainItem IPC must be bounded').toBeLessThanOrEqual(EXPLAIN_CALL_BOUND);
    expect(metrics.previewIds, 'compact selection must be sent to preview_clean').toBeLessThanOrEqual(PREVIEW_ID_BOUND);
    expect(metrics.payloadIdsMatch, 'compact payload must expand to the exact selection').toBe(true);
    expect(metrics.maxMainThreadGapMs, 'main thread must not block').toBeGreaterThan(0);
    expect(metrics.maxMainThreadGapMs, 'main thread must not block').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(metrics.determinateProgressSeen, 'determinate progress must be visible during preparation').toBe(true);
    expect(elapsedMs, 'destructive CTA must be enabled promptly').toBeLessThan(CTA_BOUND_MS);
  });
});
