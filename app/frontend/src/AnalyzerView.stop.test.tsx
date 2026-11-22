import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AnalyzerView } from './components/AnalyzerView';
import { useAnalyzerWorkflow } from './components/analyzer/useAnalyzerWorkflow';
import type { AnalyzerProgressEvent, Settings, StartAnalyzerResult, SweeperClient } from './api/client';

function rootNode() {
  return {
    id: 'root',
    name: 'C:\\',
    path: 'C:\\',
    isDir: true,
    sizeBytes: '0',
    itemCount: '0',
    risk: 'Safe' as const,
    description: 'root',
    reason: null,
    tier: null,
    targetId: null,
    isLinkPoint: false,
    isCloudPlaceholder: false,
    displayOnly: false,
    selectable: false,
    children: [] as any[],
  };
}

function renderView(client: SweeperClient) {
  return render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as Settings} onOpenLog={() => {}} />);
}

function gatedAnalyzer(input: { cancelled: boolean }) {
  let finish!: (result: StartAnalyzerResult) => void;
  const gate = new Promise<StartAnalyzerResult>((resolve) => { finish = resolve; });
  const startAnalyzer = vi.fn(async (_root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult> => {
    onProgress?.({ sessionId: 'an-stop-1', scannedDirs: '3', scannedFiles: '2', scannedBytes: '512', currentPath: 'C:\\Temp' });
    return gate;
  });
  const cancelledResult = {
    sessionId: 'an-stop-1',
    rootPath: 'C:\\',
    totalBytes: '0',
    totalItems: '0',
    scannedDirs: '3',
    scannedFiles: '2',
    cancelled: input.cancelled,
    warnings: [],
    root: rootNode(),
  } as StartAnalyzerResult;
  return { startAnalyzer, finish: () => finish(cancelledResult) };
}

async function beginAnalysis() {
  const user = userEvent.setup();
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findAllByRole('button', { name: 'Stop analysis' });
}

describe('AnalyzerView Stop convergence', () => {
  it('Stop clears the analysis flag, progress and session, and ignores the late worker result', async () => {
    const { startAnalyzer, finish } = gatedAnalyzer({ cancelled: false });
    const cancelAnalyzer = vi.fn(async (_sessionId: string) => undefined);
    renderView({ startAnalyzer, cancelAnalyzer } as unknown as SweeperClient);

    await beginAnalysis();
    await userEvent.setup().click(screen.getAllByRole('button', { name: 'Stop analysis' })[0]);

    await waitFor(() => expect(screen.queryAllByRole('button', { name: 'Stop analysis' })).toHaveLength(0));
    expect(screen.queryByText(/Analyzing C:\\/)).not.toBeInTheDocument();
    expect(screen.queryByRole('heading', { name: 'Analyzer Tree' })).not.toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Analysis stopped.');
    expect(cancelAnalyzer).toHaveBeenCalledTimes(1);
    expect(cancelAnalyzer.mock.calls[0]).toEqual(['an-stop-1']);

    finish();
    await act(async () => { await Promise.resolve(); });
    expect(screen.queryByRole('heading', { name: 'Analyzer Tree' })).not.toBeInTheDocument();
  });

  it('repeated Stop presses submit exactly one cancel', async () => {
    const { startAnalyzer } = gatedAnalyzer({ cancelled: true });
    let releaseCancel!: () => void;
    const cancelGate = new Promise<void>((resolve) => { releaseCancel = resolve; });
    const cancelAnalyzer = vi.fn(async (_sessionId: string) => { await cancelGate; });
    renderView({ startAnalyzer, cancelAnalyzer } as unknown as SweeperClient);

    await beginAnalysis();
    const stop = screen.getAllByRole('button', { name: 'Stop analysis' })[0];
    act(() => {
      fireEvent.click(stop);
      fireEvent.click(stop);
    });

    expect(cancelAnalyzer).toHaveBeenCalledTimes(1);
    releaseCancel();
  });

  it('Stop clears analyzedPath and startedAt as well as the rest of the run state', async () => {
    let finishSecond!: (result: StartAnalyzerResult) => void;
    const secondGate = new Promise<StartAnalyzerResult>((resolve) => { finishSecond = resolve; });
    let calls = 0;
    const completed = {
      sessionId: 'an-first',
      rootPath: 'C:\\Temp\\analyzed',
      totalBytes: '10',
      totalItems: '1',
      scannedDirs: '1',
      scannedFiles: '1',
      cancelled: false,
      warnings: [],
      root: rootNode(),
    } as StartAnalyzerResult;
    const client = {
      startAnalyzer: async (_root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult> => {
        calls += 1;
        if (calls === 1) return completed;
        onProgress?.({ sessionId: 'an-stop-2', scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: 'C:\\' });
        return secondGate;
      },
      cancelAnalyzer: vi.fn(async (_sessionId: string) => undefined),
      explainItem: async (itemId: string) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' }),
    } as unknown as SweeperClient;
    const hook = renderHook(() => useAnalyzerWorkflow({ client, settings: { allowDeleteUnknown: false } as Settings }));

    await act(async () => { await hook.result.current.runAnalyzer(); });
    expect(hook.result.current.analyzedPath).toBe('C:\\Temp\\analyzed');
    expect(hook.result.current.startedAt).not.toBeNull();

    await act(async () => { void hook.result.current.runAnalyzer(); });
    expect(hook.result.current.analyzing).toBe(true);
    expect(hook.result.current.runningSessionId).toBe('an-stop-2');

    await act(async () => { await hook.result.current.stopAnalyzer(); });

    expect(hook.result.current.analyzedPath).toBeNull();
    expect(hook.result.current.startedAt).toBeNull();
    expect(hook.result.current.analyzing).toBe(false);
    expect(hook.result.current.progress).toBeNull();
    expect(hook.result.current.sessionId).toBeNull();
    expect(hook.result.current.result).toBeNull();
    finishSecond({ ...completed, cancelled: true });
  });

  it('a late cancel rejection does not clobber a newer run messages', async () => {
    let rejectCancel!: (reason: unknown) => void;
    const cancelGate = new Promise<void>((_resolve, reject) => { rejectCancel = reject; });
    const cancelAnalyzer = vi.fn(async (_sessionId: string) => { await cancelGate; });
    let calls = 0;
    const client = {
      startAnalyzer: async (_root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult> => {
        calls += 1;
        onProgress?.({ sessionId: `an-late-${calls}`, scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: 'C:\\Temp' });
        return new Promise<StartAnalyzerResult>(() => undefined);
      },
      cancelAnalyzer,
    } as unknown as SweeperClient;
    renderView(client);

    const user = userEvent.setup();
    await beginAnalysis();
    await user.click(screen.getAllByRole('button', { name: 'Stop analysis' })[0]);
    expect(cancelAnalyzer).toHaveBeenCalledWith('an-late-1');

    // A new run starts while the first cancel is still pending.
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findAllByRole('button', { name: 'Stop analysis' });

    await act(async () => {
      rejectCancel(new Error('cancel failed after new run started'));
      await cancelGate.catch(() => undefined);
    });

    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Stop analysis' }).length).toBeGreaterThan(0);
  });
});
