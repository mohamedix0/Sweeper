import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { createMockController } from '../../api/mock';
import type { ContractError, ScanProgressEvent, SweeperClient, TierDto } from '../../api/client';
import { makeAnalyzerNode, makeAnalyzerResult, makeScanResults, makeSettings } from '../../test/fixtures';
import { ScanAndPreviewView } from '../ScanAndPreviewView';
import { isScanCancellation } from './useScanWorkflowRuntime';

function makeClient(overrides: Partial<SweeperClient> = {}): SweeperClient {
  const base: SweeperClient = {
    startScan: async () => ({ sessionId: 'scan-cancel-1' }),
    getScanResults: async () => makeScanResults({ sessionId: 'scan-cancel-1', targets: [] }),
    previewClean: async () => ({
      previewToken: 'pt',
      breakdown: [],
      totalSelectedCount: '0',
      totalSelectedBytes: '0',
    }),
    executeClean: async () => ({
      bytesFreed: '0',
      deletedCount: '0',
      skippedCount: '0',
      skippedReasons: [],
      aborted: false,
      abortReason: null,
      logId: null,
    }),
    startAnalyzer: async () =>
      makeAnalyzerResult({ root: makeAnalyzerNode({ id: 'root', name: 'C:', path: 'C:\\', isDir: true }) }),
    cancelAnalyzer: async () => undefined,
    cancelScan: async () => undefined,
    explainItem: async (itemId) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' }),
    getSettings: async () => makeSettings(),
    updateSettings: async (settings) => settings,
    getElevationStatus: async () => ({ isElevated: false }),
    relaunchElevated: async () => ({ launched: false }),
    validateCustomPath: async (path) => ({ path, allowed: false, reason: 'test' }),
    listLogs: async () => [],
    readLog: async (logId) => ({ id: logId, content: '' }),
  };
  return { ...base, ...overrides };
}

function renderView(client: SweeperClient) {
  return render(
    <ScanAndPreviewView
      client={client}
      settings={makeSettings()}
      settingsVersion={0}
      elevationStatus={{ isElevated: false }}
      elevationError={null}
      onOpenLog={() => undefined}
    />,
  );
}

const cancelledError = {
  code: 'scanCancelled',
  details: { session_id: 'scan-cancel-1' },
} satisfies ContractError;

describe('Scan cancellation', () => {
  it('a running Scan offers an accessible Stop that cancels by session id only and shows the cancelled state', async () => {
    const user = userEvent.setup();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const cancelScan = vi.fn(async (_sessionId: string) => {
      release();
    });
    const startScan = vi.fn(
      async (_tiers: TierDto[] | null, onProgress?: (event: ScanProgressEvent) => void) => {
        onProgress?.({ kind: 'started', session_id: 'scan-cancel-1' });
        onProgress?.({
          kind: 'progress',
          session_id: 'scan-cancel-1',
          completed_targets: 1,
          total_targets: 3,
          current_target_id: 't1',
          current_target_name: 'User Temp',
          scanned_items: '12',
          scanned_bytes: '4096',
        });
        await gate;
        onProgress?.({ kind: 'cancelled', session_id: 'scan-cancel-1' });
        throw cancelledError;
      },
    );

    renderView(makeClient({ startScan, cancelScan }));
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));

    const stop = await screen.findByRole('button', { name: 'Stop Scan' });
    expect(stop).toBeVisible();
    expect(stop).toHaveAttribute('type', 'button');

    await user.click(stop);
    expect(cancelScan).toHaveBeenCalledTimes(1);
    expect(cancelScan.mock.calls[0]).toEqual(['scan-cancel-1']);

    expect(await screen.findByRole('alert')).toHaveTextContent('Scan cancelled.');
    expect(screen.queryByText('Stopping Scan…')).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Stop Scan' })).not.toBeInTheDocument(),
    );
  });

  it('does not re-apply the Stop notice when the Scan finishes before cancelScan resolves', async () => {
    const user = userEvent.setup();
    let releaseScan!: () => void;
    const scanGate = new Promise<void>((resolve) => {
      releaseScan = resolve;
    });
    let releaseCancel!: () => void;
    const cancelGate = new Promise<void>((resolve) => {
      releaseCancel = resolve;
    });
    const startScan = vi.fn(
      async (_tiers: TierDto[] | null, onProgress?: (event: ScanProgressEvent) => void) => {
        onProgress?.({ kind: 'started', session_id: 'scan-cancel-1' });
        await scanGate;
        return { sessionId: 'scan-cancel-1' };
      },
    );
    const cancelScan = vi.fn(async (_sessionId: string) => {
      await cancelGate;
    });

    renderView(makeClient({ startScan, cancelScan }));
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await user.click(await screen.findByRole('button', { name: 'Stop Scan' }));
    await waitFor(() => expect(cancelScan).toHaveBeenCalledTimes(1));

    // The Scan reaches a terminal state while the Stop IPC is still pending.
    await act(async () => {
      releaseScan();
    });
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Stop Scan' })).not.toBeInTheDocument(),
    );

    // The late Stop resolution must not resurrect the request-specific notice.
    await act(async () => {
      releaseCancel();
    });
    expect(screen.queryByText('Stopping Scan…')).not.toBeInTheDocument();
  });

  it('submits a Stop clicked before the first progress event once the id arrives', async () => {
    const user = userEvent.setup();
    let emitId!: () => void;
    const idGate = new Promise<void>((resolve) => {
      emitId = resolve;
    });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const cancelScan = vi.fn(async (_sessionId: string) => {
      release();
    });
    const startScan = vi.fn(
      async (_tiers: TierDto[] | null, onProgress?: (event: ScanProgressEvent) => void) => {
        await idGate;
        onProgress?.({ kind: 'started', session_id: 'scan-cancel-1' });
        await gate;
        return { sessionId: 'scan-cancel-1' };
      },
    );

    renderView(makeClient({ startScan, cancelScan }));
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await user.click(await screen.findByRole('button', { name: 'Stop Scan' }));
    expect(cancelScan).not.toHaveBeenCalled();

    emitId();
    await waitFor(() => expect(cancelScan).toHaveBeenCalledTimes(1));
    expect(cancelScan.mock.calls[0]).toEqual(['scan-cancel-1']);
  });

  it('reports a failed Stop request without changing the running Scan', async () => {
    const user = userEvent.setup();
    const never = new Promise<void>(() => undefined);
    const cancelScan = vi.fn(async () => {
      throw new Error('IPC unavailable');
    });
    renderView(
      makeClient({
        startScan: async (_tiers, onProgress) => {
          onProgress?.({ kind: 'started', session_id: 'scan-cancel-1' });
          await never;
          return { sessionId: 'scan-cancel-1' };
        },
        cancelScan,
      }),
    );

    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await user.click(await screen.findByRole('button', { name: 'Stop Scan' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('Failed to stop Scan.');
    expect(screen.getByRole('button', { name: 'Stop Scan' })).toBeVisible();
  });

  it('reflects a host-requested cancellation of in-flight Scans while the window is open', async () => {
    const user = userEvent.setup();
    const client = createMockController();
    client.setScanStepDelayMs(60);
    renderView(client);

    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('button', { name: 'Stop Scan' });

    await act(async () => {
      client.cancelActiveScans();
    });

    expect(await screen.findByRole('alert')).toHaveTextContent('Scan cancelled.');
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'Stop Scan' })).not.toBeInTheDocument(),
    );
  });

  it('recognises cancellation by the typed code, not by an English reason string', () => {
    expect(isScanCancellation(cancelledError)).toBe(true);
    expect(
      isScanCancellation({ code: 'internal', details: { reason: 'scan cancelled' } } satisfies ContractError),
    ).toBe(false);
    expect(isScanCancellation({ code: 'invalidInput', details: { reason: 'nope' } } satisfies ContractError)).toBe(false);
  });
});
