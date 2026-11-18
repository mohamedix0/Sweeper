import { describe, expect, it, vi } from 'vitest';
import type { SweeperClient } from '../../api/client';
import { MAX_COMPACT_IDS } from './previewPerf';
import { clearScanSelection, runScanPreview } from './useScanViewActions';

function fakeWorkflow() {
  const previewRevisionRef = { current: 0 };
  const activePreviewRequestRef = { current: 0 };
  const setPreviewLoading = vi.fn();
  return {
    isSessionExpired: false,
    activePreviewRequestRef,
    previewRevisionRef,
    sessionId: 'scan-1',
    setError: vi.fn(),
    setInfo: vi.fn(),
    setPreviewLoading,
    setCleanSummary: vi.fn(),
    setCleanProgress: vi.fn(),
    setPreviewProgress: vi.fn(),
    setPreview: vi.fn(),
    setSelectedIds: vi.fn(),
    // Mirror useInvalidate: advancing both refs is what makes an in-flight
    // Preview stale and is the signal mid-preparation cancellation relies on.
    invalidatePreviewRequest: vi.fn(() => {
      previewRevisionRef.current += 1;
      activePreviewRequestRef.current += 1;
      setPreviewLoading(false);
    }),
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

  it('stops preparing IDs and sends no Preview IPC when the selection is invalidated mid-preparation', async () => {
    const w = fakeWorkflow();
    const previewClean = vi.fn(() => Promise.resolve({ previewToken: 'tok' }));
    const selected = new Set<string>();
    for (let i = 1; i <= 5_000; i += 1) selected.add(`user-temp::${i}`); // 5 chunks

    // Simulate the user clicking "Unselect all" as soon as the first chunk
    // reports progress: invalidatePreview advances both refs and clears status.
    let cancelled = false;
    w.setPreviewProgress.mockImplementation((value: { completed: number; total: number } | null) => {
      if (value && !cancelled) {
        cancelled = true;
        clearScanSelection({ w });
      }
    });

    await runScanPreview({
      w,
      client: fakeClient(previewClean),
      sel: selected,
      requiresConfirmation: false,
      sessionId: 'scan-1',
      confirmText: '',
    });

    expect(cancelled).toBe(true);
    expect(previewClean).not.toHaveBeenCalled();
    // Invalidation clears the preview with null; the stale action must never
    // commit a new (non-null) Preview result after cancellation.
    expect(w.setPreview.mock.calls.filter(([preview]) => preview !== null)).toHaveLength(0);
    // Only the first chunk wrote progress; no stale progress/state write after
    // cancellation, and preparation stopped before collecting the other chunks.
    expect(w.setPreviewProgress.mock.calls.filter(([progress]) => progress !== null)).toHaveLength(1);
    expect(w.setPreviewProgress).toHaveBeenLastCalledWith(null);
  });
});
