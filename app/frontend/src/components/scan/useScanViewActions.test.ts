import { describe, expect, it, vi } from 'vitest';
import type { SweeperClient } from '../../api/client';
import { MAX_COMPACT_IDS } from './previewPerf';
import { clearScanSelection, runScanPreview } from './useScanViewActions';

function fakeWorkflow() {
  return {
    isSessionExpired: false,
    activePreviewRequestRef: { current: 0 },
    previewRevisionRef: { current: 0 },
    sessionId: 'scan-1',
    setError: vi.fn(),
    setInfo: vi.fn(),
    setPreviewLoading: vi.fn(),
    setCleanSummary: vi.fn(),
    setCleanProgress: vi.fn(),
    setPreviewProgress: vi.fn(),
    setPreview: vi.fn(),
    setSelectedIds: vi.fn(),
    invalidatePreviewRequest: vi.fn(),
    resetForInvalidSession: vi.fn(),
  };
}

function fakeClient(previewClean: ReturnType<typeof vi.fn>) {
  return { previewClean } as unknown as SweeperClient;
}

describe('preview invalidation', () => {
  it('clears preparation progress when a selection change invalidates the Preview', () => {
    const w = fakeWorkflow();
    w.setPreviewProgress({ completed: 3, total: 10 });
    clearScanSelection({ w });
    expect(w.invalidatePreviewRequest).toHaveBeenCalledTimes(1);
    expect(w.setPreviewProgress).toHaveBeenLastCalledWith(null);
  });
});

describe('runScanPreview preparation safety', () => {
  it('rejects an oversized selection with a clear message instead of sending an undecodable descriptor', async () => {
    const w = fakeWorkflow();
    const previewClean = vi.fn();
    const selected = new Set<string>();
    for (let i = 1; i <= MAX_COMPACT_IDS + 1; i += 1) selected.add(`user-temp::${i}`);

    await runScanPreview({ w, client: fakeClient(previewClean), sel: selected, requiresConfirmation: false, sessionId: 'scan-1', confirmText: '' });

    expect(previewClean).not.toHaveBeenCalled();
    expect(w.setError).toHaveBeenCalledWith(expect.stringMatching(/at most 1,000,000 items/));
    expect(w.setPreview).not.toHaveBeenCalled();
  }, 20_000);

  it('clears preparation progress before waiting on the Preview API', async () => {
    const w = fakeWorkflow();
    let resolvePreview!: (value: unknown) => void;
    const previewPromise = new Promise((resolve) => { resolvePreview = resolve; });
    const previewClean = vi.fn(() => previewPromise);
    const pending = runScanPreview({
      w,
      client: fakeClient(previewClean),
      sel: new Set(['user-temp::1']),
      requiresConfirmation: false,
      sessionId: 'scan-1',
      confirmText: '',
    });

    await vi.waitFor(() => expect(previewClean).toHaveBeenCalled());
    expect(w.setPreviewProgress).toHaveBeenLastCalledWith(null);
    resolvePreview({ previewToken: 'tok', totalSelectedCount: '1', totalSelectedBytes: '100' });
    await pending;
  });
});
