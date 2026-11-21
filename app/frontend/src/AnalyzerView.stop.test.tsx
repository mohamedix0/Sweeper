import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AnalyzerView } from './components/AnalyzerView';
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
});
