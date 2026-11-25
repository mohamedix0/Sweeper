import { describe, expect, it } from 'vitest';
import {
  MAX_MAIN_THREAD_GAP_MS,
  SPIKE_BAND_MS,
  WINDOW_FRAMES,
  summarizeGaps,
} from './previewGapCalibration';

/** ~58-62 ms typical preview-phase frames (no scheduling outlier). */
function healthyWindow(): number[] {
  return Array.from({ length: WINDOW_FRAMES }, (_, i) => 58 + (i % 5));
}

/** Insert one sample into an otherwise healthy window. */
function withOneFrame(frameMs: number): number[] {
  const frames = healthyWindow();
  return [...frames.slice(0, 12), frameMs, ...frames.slice(12)];
}

/** True when the calibrated metric reports the main thread as blocked. */
function blocksMainThread(frames: number[]): boolean {
  const gap = summarizeGaps(frames).effectiveGapMs;
  return gap <= 0 || gap >= MAX_MAIN_THREAD_GAP_MS;
}

describe('T5 large-selection Preview gap calibration', () => {
  it('classifies a healthy preview window as responsive', () => {
    const summary = summarizeGaps(healthyWindow());
    expect(summary.sampleCount).toBe(WINDOW_FRAMES);
    expect(summary.effectiveGapMs).toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(blocksMainThread(healthyWindow())).toBe(false);
  });

  it('rejects the documented single 206 ms scheduling outlier', () => {
    const summary = summarizeGaps(withOneFrame(206));
    expect(summary.maxMs, 'the old single-max metric would fail here').toBe(206);
    expect(summary.maxMs).toBeGreaterThanOrEqual(SPIKE_BAND_MS);
    expect(summary.calibratedGapMs, 'p95 trims the lone spike').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(summary.effectiveGapMs, 'a lone outlier must not fail the run').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(blocksMainThread(withOneFrame(206))).toBe(false);
  });

  it('catches a single ~90 ms mid-band stall despite the p95 rank cap', () => {
    // Regression for the original finding: capping the p95 rank at
    // sorted.length - 2 discards the single largest frame for EVERY window of
    // two or more samples, so p95 alone can no longer see one mid-band stall.
    const summary = summarizeGaps(withOneFrame(90));
    expect(summary.maxMs).toBe(90);
    expect(summary.maxMs, 'the stall is below the scheduling spike band').toBeLessThan(SPIKE_BAND_MS);
    expect(summary.calibratedGapMs, 'p95 alone trims the single stall (the bug)').toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(summary.effectiveGapMs, 'the raw-max gate must still catch one ~90 ms frame')
      .toBeGreaterThanOrEqual(MAX_MAIN_THREAD_GAP_MS);
    expect(blocksMainThread(withOneFrame(90))).toBe(true);
  });

  it('catches a sustained ~90 ms stall at the unchanged 80 ms bound', () => {
    const frames = healthyWindow().map((gap, i) => (i >= 8 ? 92 : gap));
    const summary = summarizeGaps(frames);
    expect(summary.effectiveGapMs).toBeGreaterThanOrEqual(MAX_MAIN_THREAD_GAP_MS);
    expect(blocksMainThread(frames)).toBe(true);
  });

  it('catches a sustained stall even when a scheduling outlier is also present', () => {
    const stalled = healthyWindow().map((gap, i) => (i >= 8 ? 92 : gap));
    const frames = [...stalled.slice(0, 12), 206, ...stalled.slice(12)];
    const summary = summarizeGaps(frames);
    expect(summary.maxMs, 'the outlier pushes raw max into the spike band').toBeGreaterThanOrEqual(SPIKE_BAND_MS);
    expect(summary.effectiveGapMs, 'p95 still sees the sustained stall past the spike')
      .toBeGreaterThanOrEqual(MAX_MAIN_THREAD_GAP_MS);
    expect(blocksMainThread(frames)).toBe(true);
  });

  it('keeps sub-bound single frames below the spike band', () => {
    // Frames just below the 80 ms bound must still pass; only real stalls fail.
    const summary = summarizeGaps(withOneFrame(79));
    expect(summary.maxMs).toBeLessThan(SPIKE_BAND_MS);
    expect(summary.effectiveGapMs).toBeLessThan(MAX_MAIN_THREAD_GAP_MS);
    expect(blocksMainThread(withOneFrame(79))).toBe(false);
  });
});
