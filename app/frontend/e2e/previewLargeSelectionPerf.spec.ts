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
const MAX_MAIN_THREAD_GAP_MS = 50;
const CTA_BOUND_MS = 15_000;

type Metrics = {
  explainCalls: number;
  previewIds: number;
  maxMainThreadGapMs: number;
  progressSeenAtPreviewCall: boolean;
};

async function installGapSampler(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const g = globalThis as unknown as {
      __gapSamples: number[];
      __lastFrame: number;
      __resetGaps: () => void;
    };
    g.__gapSamples = [];
    g.__lastFrame = performance.now();
    g.__resetGaps = () => {
      g.__gapSamples = [];
      g.__lastFrame = performance.now();
    };
    const tick = (now: number) => {
      g.__gapSamples.push(now - g.__lastFrame);
      g.__lastFrame = now;
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
    const metrics = (window as unknown as { __perf: Record<string, number | boolean> }).__perf ?? {};
    (window as unknown as { __perf: Record<string, number | boolean> }).__perf = metrics;
    metrics.explainCalls = 0;
    metrics.previewIds = 0;
    metrics.previewCalls = 0;

    const items = Array.from({ length: n }, (_, idx) => ({
      id: `user-temp::${idx + 1}`,
      targetId: 'user-temp',
      path: `Z:\\Synthetic\\Alice\\item-${idx + 1}`,
      size: '100',
      mtimeUnixSecs: '1700000000',
      ageEligible: true,
    }));
    const bytes = String(n * 100);
    const target = {
      targetId: 'user-temp', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe',
      resolvedPaths: ['Z:\\Synthetic\\Alice'], scanned: true, statusReason: null,
      totalBytes: bytes, eligibleBytes: bytes, fileCount: String(n), eligibleCount: String(n),
      skippedCount: '0', items,
    };

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
      metrics.progressSeenAtPreviewCall = Boolean(document.querySelector('.preview-preparing'));
      return {
        previewToken: 'perf-token',
        breakdown: [{ targetId: 'user-temp', targetName: 'User Temp', selectedCount: String(n), selectedBytes: bytes }],
        totalSelectedCount: String(n),
        totalSelectedBytes: bytes,
      };
    };
  }, count);
}

async function readMetrics(page: Page): Promise<Metrics> {
  return page.evaluate(() => {
    const g = globalThis as unknown as { __gapSamples: number[] };
    const p = (window as unknown as { __perf: Record<string, number | boolean> }).__perf;
    const gaps = g.__gapSamples.slice().sort((a, b) => a - b);
    return {
      explainCalls: Number(p.explainCalls),
      previewIds: Number(p.previewIds),
      maxMainThreadGapMs: gaps.length > 0 ? gaps[gaps.length - 1] : 0,
      progressSeenAtPreviewCall: Boolean(p.progressSeenAtPreviewCall),
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

    const cta = page.getByRole('button', { name: 'Clean Permanently' });
    await expect(cta).toBeEnabled({ timeout: CTA_BOUND_MS });
    const elapsedMs = Date.now() - started;

    const metrics = await readMetrics(page);
    console.log('T3 preview perf @20k', { ...metrics, elapsedMs });
    expect(metrics.explainCalls, 'explainItem IPC must be bounded').toBeLessThanOrEqual(EXPLAIN_CALL_BOUND);
    expect(metrics.previewIds, 'compact selection must be sent to preview_clean').toBeLessThanOrEqual(PREVIEW_ID_BOUND);
    expect(metrics.maxMainThreadGapMs, 'main thread must not block').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(metrics.progressSeenAtPreviewCall, 'determinate progress must be visible during preparation').toBe(true);
    expect(elapsedMs, 'destructive CTA must be enabled promptly').toBeLessThan(CTA_BOUND_MS);
  });
});
