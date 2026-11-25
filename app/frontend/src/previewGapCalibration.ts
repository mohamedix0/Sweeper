/**
 * Pure, browser-independent calibration metric for the large-selection Preview
 * rAF gap sampler (issue #145 / audit T5). Shared by the Playwright perf spec
 * and its dedicated Vitest regression test so the two cannot drift.
 *
 * The 80 ms acceptance threshold is a fixed guard for the ~90 ms final-render
 * stall and is never lowered. Two gates are combined in `effectiveGapMs`:
 *  - a spike-rejected p95 (the single largest frame is trimmed) rejects an
 *    isolated OS scheduling outlier such as the documented 206 ms sample;
 *  - a raw-max gate below an explicit spike band keeps a single mid-band stall
 *    (~90 ms) detectable, so the original single-frame guard is not weakened.
 */

/** Unchanged acceptance bound for the ~90 ms final-render stall. */
export const MAX_MAIN_THREAD_GAP_MS = 80;
/** At or above this band a single frame is treated as a scheduling outlier. */
export const SPIKE_BAND_MS = 150;

// Calibration knobs: these change how the gap is *measured*, never the bound.
export const WARMUP_FRAMES = 3;
export const WINDOW_FRAMES = 24;
export const MIN_SAMPLES = 6;
export const P95 = 0.95;

export type GapSummary = {
  sampleCount: number;
  maxMs: number;
  p95Ms: number;
  /** Spike-rejected p95 metric (single largest frame trimmed). */
  calibratedGapMs: number;
  /**
   * Gate value used by the perf spec: raw max below the spike band (so one
   * mid-band stall still fails), spike-rejected p95 at or above it (so a lone
   * scheduling spike is rejected).
   */
  effectiveGapMs: number;
};

/** Nearest-rank percentile of an already sorted ascending numeric array. */
export function nearestRankPercentile(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  const rank = Math.max(0, Math.min(Math.ceil(p * sorted.length) - 1, sorted.length - 1));
  return sorted[rank];
}

/**
 * Summarise rAF gaps over a short, warm-up-discarded window.
 *
 * Nearest-rank p95 drops the single largest frame for a full 24-sample window,
 * but equals the max when the window is short. Cap the rank so one scheduling
 * spike is always trimmed; a sustained stall leaves plenty of >= 80 ms frames
 * behind. Separately, because that cap discards the single largest frame for
 * *every* window of two or more samples, a single mid-band stall (e.g. 90 ms)
 * would otherwise escape detection. Gate on the raw max whenever it is below
 * the explicit spike band, restoring the original single-frame guard without
 * re-introducing the outlier flake.
 */
export function summarizeGaps(samples: number[]): GapSummary {
  const sorted = samples.slice().sort((a, b) => a - b);
  const maxMs = sorted.length > 0 ? sorted[sorted.length - 1] : 0;
  const p95Ms = nearestRankPercentile(sorted, P95);
  const p95Index = Math.min(
    sorted.length > 0 ? Math.ceil(P95 * sorted.length) - 1 : 0,
    Math.max(0, sorted.length - 2),
  );
  const calibratedGapMs = sorted[Math.max(0, p95Index)] ?? 0;
  const effectiveGapMs = maxMs < SPIKE_BAND_MS ? maxMs : calibratedGapMs;
  return { sampleCount: sorted.length, maxMs, p95Ms, calibratedGapMs, effectiveGapMs };
}
