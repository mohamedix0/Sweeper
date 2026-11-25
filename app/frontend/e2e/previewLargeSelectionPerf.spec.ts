import { expect, test, type Page } from '@playwright/test';
import {
  MAX_MAIN_THREAD_GAP_MS,
  MIN_SAMPLES,
  WARMUP_FRAMES,
  WINDOW_FRAMES,
  summarizeGaps,
  type GapSummary,
} from '../src/previewGapCalibration';

/**
 * R4 regression (#61): generating a Preview for a very large selection must
 * keep the UI responsive. All data is synthetic (Alice-style fixture IDs, no
 * real paths); the mock backend is instrumented in the browser only, and the
 * Edge channel comes from playwright.config.ts. No real Clean is performed.
 *
 * T5 (#145) calibration: the original metric took a single max over every rAF
 * delta in an unbounded window. One OS scheduling hiccup under parallel worker
 * load (observed 206 ms) failed an otherwise-healthy run. We now discard
 * warm-up frames, keep a bounded window of recent preview-phase frames, assert
 * on p95 instead of the single max, and retry the whole measurement locally.
 * The 80 ms threshold is unchanged. A single mid-band stall is still caught by
 * a raw-max gate below an explicit spike band, so the ~90 ms guard is not
 * weakened; a lone scheduling spike above that band is rejected by p95. A
 * real loss of responsiveness also starves the sampler (MIN_SAMPLES).
 * The pure metric lives in src/previewGapCalibration.ts and has a dedicated
 * Vitest regression suite next to it.
 */
const ITEM_COUNT = 20_000;
const EXPLAIN_CALL_BOUND = 32;
const PREVIEW_ID_BOUND = 8;
// The acceptance bound and calibration knobs are imported from the shared
// module above; only E2E-flow constants stay local.
const CTA_BOUND_MS = 15_000;
const MAX_ATTEMPTS = 3;

type Metrics = {
  explainCalls: number;
  previewCalls: number;
  previewIds: number;
  payloadIdsMatch: boolean;
  gaps: number[];
  determinateProgressSeen: boolean;
};

type SamplerConfig = {
  expectedMax: number;
  warmupFrames: number;
  windowFrames: number;
};

async function installGapSampler(page: Page, config: SamplerConfig): Promise<void> {
  await page.addInitScript((cfg) => {
    const g = globalThis as unknown as {
      __gapSamples: number[];
      __lastFrame: number;
      __warmupRemaining: number;
      // Monotonic count of recorded samples: the bounded buffer length stops
      // growing once full, so it cannot signal "a new frame arrived" (#145).
      __sampleCount: number;
      __resetGaps: () => void;
      __perf: Record<string, number | boolean>;
    };
    g.__gapSamples = [];
    g.__lastFrame = 0;
    g.__warmupRemaining = cfg.warmupFrames;
    g.__sampleCount = 0;
    g.__perf = {};
    g.__resetGaps = () => {
      g.__gapSamples = [];
      g.__sampleCount = 0;
      // 0 means "no baseline yet": the first frame after a reset spans the
      // click/effect scheduling, not a renderer stall, so it is not sampled.
      g.__lastFrame = 0;
      // Warm-up frames bridge the click/effect scheduling and layout before a
      // gap becomes representative of Preview work.
      g.__warmupRemaining = cfg.warmupFrames;
    };
    const isVisible = (el: Element): boolean => {
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return false;
      const style = getComputedStyle(el);
      return style.visibility !== 'hidden' && style.display !== 'none';
    };
    // Record a determinate progress meter *while preparation is running*, on
    // the same animation frame as the gap sampler. Checking for the wrapper at
    // the Preview IPC call is not evidence that the meter ever showed a real
    // value (and progress is cleared before that call once encoding finishes).
    // Require a VISIBLE meter whose max equals the fixture selection: a hidden
    // meter or one with the wrong maximum must not satisfy the assertion.
    const recordDeterminateProgress = () => {
      const meter = document.querySelector('.preview-preparing progress');
      if (!meter || !isVisible(meter)) return;
      const max = Number((meter as HTMLProgressElement).max);
      if (max !== cfg.expectedMax) return;
      const value = Number((meter as HTMLProgressElement).value);
      if (value <= 0 || value >= max) return;
      g.__perf.determinateProgressSeen = true;
    };
    // Per-frame gap bookkeeping: discard warm-up frames, then keep only the
    // most recent preview-phase frames. It reads __lastFrame, so it must run
    // before the callback advances that baseline. Extracted to keep the rAF
    // callback a straight-line sequence (CodeScene: Bumpy Road Ahead on tick).
    const recordFrameGap = (now: number) => {
      if (g.__lastFrame <= 0) return;
      if (g.__warmupRemaining > 0) {
        g.__warmupRemaining -= 1;
        return;
      }
      g.__gapSamples.push(now - g.__lastFrame);
      g.__sampleCount += 1;
      if (g.__gapSamples.length > cfg.windowFrames) g.__gapSamples.shift();
    };
    const tick = (now: number) => {
      recordFrameGap(now);
      g.__lastFrame = now;
      recordDeterminateProgress();
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }, config);
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
    if (g.__gapSamples.length === 0) throw new Error('animation-frame sampler recorded no frames');
    return {
      explainCalls: Number(p.explainCalls),
      previewCalls: Number(p.previewCalls),
      previewIds: Number(p.previewIds),
      payloadIdsMatch: Boolean(p.payloadIdsMatch),
      gaps: g.__gapSamples.slice(),
      determinateProgressSeen: Boolean(p.determinateProgressSeen),
    };
  });
}

/** One clean measurement on a freshly navigated page. */
async function measureOnce(page: Page, attempt: number): Promise<{ metrics: Metrics; elapsedMs: number }> {
  if (attempt === 1) await page.goto('/');
  else await page.reload();
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
  const framesBeforeEnable = await page.evaluate(() => (globalThis as unknown as { __sampleCount: number }).__sampleCount);
  await expect(cta).toBeEnabled({ timeout: CTA_BOUND_MS });
  // The final render that enables the CTA can itself occupy a frame interval.
  // Wait for a full MIN_SAMPLES of post-enable frames so the gap summary is
  // statistically meaningful. Use the monotonic counter: the bounded buffer
  // length plateaus once full and can never advance the wait (#145).
  await page.waitForFunction(
    ({ before, min }) => (globalThis as unknown as { __sampleCount: number }).__sampleCount >= before + min,
    { before: framesBeforeEnable, min: MIN_SAMPLES },
  );
  const elapsedMs = Date.now() - started;

  return { metrics: await readMetrics(page), elapsedMs };
}

test.describe('Sweeper Preview responsiveness at large selections (mock sandbox)', () => {
  test('20k selection: bounded IPC, compact selection, determinate progress, responsive UI', async ({ page }) => {
    await installGapSampler(page, {
      expectedMax: ITEM_COUNT,
      warmupFrames: WARMUP_FRAMES,
      windowFrames: WINDOW_FRAMES,
    });

    let last: GapSummary | undefined;
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt += 1) {
      const { metrics, elapsedMs } = await measureOnce(page, attempt);
      const summary = summarizeGaps(metrics.gaps);

      // Deterministic invariants: a failure here is a real regression, not contention.
      expect(metrics.explainCalls, 'explainItem IPC must be bounded').toBeLessThanOrEqual(EXPLAIN_CALL_BOUND);
      expect(metrics.previewCalls, 'one Review click must send exactly one Preview request').toBe(1);
      expect(metrics.previewIds, 'compact selection must be sent to preview_clean').toBeLessThanOrEqual(PREVIEW_ID_BOUND);
      expect(metrics.payloadIdsMatch, 'compact payload must expand to the exact selection').toBe(true);
      expect(metrics.determinateProgressSeen, 'determinate progress must be visible during preparation').toBe(true);
      expect(elapsedMs, 'destructive CTA must be enabled promptly').toBeLessThan(CTA_BOUND_MS);
      expect(summary.sampleCount, 'sampler must record enough preview-phase frames').toBeGreaterThanOrEqual(MIN_SAMPLES);

      console.log('T3 preview perf @20k', { attempt, ...summary, elapsedMs });
      if (summary.effectiveGapMs > 0 && summary.effectiveGapMs < MAX_MAIN_THREAD_GAP_MS) return;

      last = summary;
      if (attempt < MAX_ATTEMPTS) {
        console.log(`preview perf gap ${summary.effectiveGapMs}ms >= ${MAX_MAIN_THREAD_GAP_MS}ms; local retry ${attempt}/${MAX_ATTEMPTS}`);
      }
    }

    // All local attempts failed: report the last gate gap as the failure.
    expect(last?.effectiveGapMs, `main thread must not block after ${MAX_ATTEMPTS} local attempts`).toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(last?.effectiveGapMs, 'main thread gap must be positive').toBeGreaterThan(0);
  });

  test('T5 calibration: one spike is rejected, one or many ~90 ms stalls are caught', () => {
    // ~58-62 ms typical preview frames.
    const healthy = Array.from({ length: WINDOW_FRAMES }, (_, i) => 58 + (i % 5));

    // Old single-max metric: one 206 ms scheduling spike fails the 80 ms bound.
    // The spike band classifies it as an outlier, so it must be rejected.
    const withOutlier = [...healthy.slice(0, 12), 206, ...healthy.slice(12)];
    const contaminated = summarizeGaps(withOutlier);
    expect(contaminated.maxMs, 'the old single-max metric would fail here').toBeGreaterThan(MAX_MAIN_THREAD_GAP_MS);
    expect(contaminated.effectiveGapMs, 'a single OS scheduling spike must be rejected').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);

    // A single mid-band stall is below the spike band, so the raw-max gate
    // must still catch it even though p95 trims the largest frame.
    const oneStall = [...healthy.slice(0, 12), 90, ...healthy.slice(12)];
    const singleStall = summarizeGaps(oneStall);
    expect(singleStall.calibratedGapMs, 'p95 alone trims one stall (the rank-cap bug)').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(singleStall.effectiveGapMs, 'one ~90 ms frame must still fail the guard').toBeGreaterThanOrEqual(MAX_MAIN_THREAD_GAP_MS);

    // A real loss of responsiveness shows up across many frames: p95 must stay
    // at or above the unchanged 80 ms threshold so the stall guard still trips.
    const sustainedStall = healthy.map((gap, i) => (i >= 8 ? 92 : gap));
    const stalled = summarizeGaps(sustainedStall);
    expect(stalled.effectiveGapMs, 'a sustained ~90 ms stall must still be caught').toBeGreaterThanOrEqual(MAX_MAIN_THREAD_GAP_MS);

    // One spike must not mask a stall that is also present in the window.
    const stallAndOutlier = [...sustainedStall.slice(0, 12), 206, ...sustainedStall.slice(12)];
    const both = summarizeGaps(stallAndOutlier);
    expect(both.effectiveGapMs, 'a sustained stall alongside one spike is still caught').toBeGreaterThanOrEqual(MAX_MAIN_THREAD_GAP_MS);
  });
});
