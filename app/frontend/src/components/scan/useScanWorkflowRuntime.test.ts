import { describe, expect, it, vi } from 'vitest';
import { clearForReset, prepareScanState } from './useScanWorkflowRuntime';
import type { ScanState } from './useScanWorkflowRuntime';

function fakeScanState() {
  return {
    previewRevisionRef: { current: 0 },
    activePreviewRequestRef: { current: 3 },
    inspectReqIdRef: { current: 0 },
    setPreviewLoading: vi.fn(),
    setError: vi.fn(),
    setInfo: vi.fn(),
    setPreview: vi.fn(),
    setConfirmText: vi.fn(),
    setCleanProgress: vi.fn(),
    setScanProgress: vi.fn(),
    setScanStatus: vi.fn(),
    setFocus: vi.fn(),
    setInspected: vi.fn(),
    setPreviewProgress: vi.fn(),
    setSelectedIds: vi.fn(),
    setCollapsedTargets: vi.fn(),
    setShowAllGroups: vi.fn(),
    setResults: vi.fn(),
    setCleanSummary: vi.fn(),
    setScanning: vi.fn(),
    setSessionId: vi.fn(),
    setScanStartedAt: vi.fn(),
    setScanSettingsVersion: vi.fn(),
  };
}

function asScanState(state: ReturnType<typeof fakeScanState>): ScanState {
  return state as unknown as ScanState;
}

describe('preview preparation progress lifecycle', () => {
  it('clears progress when a new Scan starts', () => {
    const state = fakeScanState();
    const invalidate = vi.fn();

    prepareScanState(asScanState(state), invalidate);

    expect(invalidate).toHaveBeenCalledTimes(1);
    expect(state.setPreviewProgress).toHaveBeenCalledWith(null);
  });

  it('clears progress on reset', () => {
    const state = fakeScanState();
    const invalidate = vi.fn();

    clearForReset(asScanState(state), invalidate);

    expect(invalidate).toHaveBeenCalledTimes(1);
    expect(state.setPreviewProgress).toHaveBeenCalledWith(null);
  });
});
