import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import type { ScanResults, SweeperClient } from './api/client';

function scanResults(sessionId: string): ScanResults {
  return {
    sessionId,
    startedUnixSecs: '1710000000',
    minAgeHours: 24,
    targets: [{
      targetId: 't-cache',
      targetName: 'Cache',
      tier: 'Tier1',
      risk: 'Safe',
      resolvedPaths: [],
      scanned: true,
      statusReason: null,
      totalBytes: '100',
      eligibleBytes: '100',
      fileCount: '1',
      eligibleCount: '1',
      skippedCount: '0',
      items: [{ id: 'cache-item-1', targetId: 't-cache', path: 'cache/entry.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }],
    }],
  } as ScanResults;
}

function previewResult(sessionId: string) {
  return {
    sessionId,
    previewToken: 'tok',
    issuedAtUnixSecs: '1710000000',
    expiresAtUnixSecs: '1710000600',
    totalBytes: '100',
    totalItems: '1',
    requiresConfirmation: false,
    requiresElevation: false,
    perTarget: [],
    breakdown: [{ targetId: 't-cache', targetName: 'Cache', selectedCount: '1', selectedBytes: '100' }],
    totalSelectedCount: '1',
    totalSelectedBytes: '100',
  };
}

function renderView(client: SweeperClient) {
  return render(
    <ScanAndPreviewView
      client={client}
      settings={{ allowDeleteUnknown: false } as never}
      settingsVersion={0}
      elevationStatus={{ isElevated: false }}
      elevationError={null}
      onOpenLog={() => {}}
    />,
  );
}

describe('ScanAndPreviewView post-Preview delete discoverability', () => {
  it('shows a visible pending reason until safety review resolves, then enables the labelled delete CTA', async () => {
    let resolveExplain!: (value: unknown) => void;
    const explainPromise = new Promise<unknown>((resolve) => { resolveExplain = resolve; });
    const executeClean = vi.fn(async () => ({
      bytesFreed: '100',
      deletedCount: '1',
      skippedCount: '0',
      skippedReasons: [],
      aborted: false,
      abortReason: null,
      logId: 'log-1',
    }));
    const client = {
      startScan: async () => ({ sessionId: 'scan-delete-cta' }),
      getScanResults: async () => scanResults('scan-delete-cta'),
      explainItem: () => explainPromise,
      previewClean: async () => previewResult('scan-delete-cta'),
      executeClean,
    } as unknown as SweeperClient;

    const user = userEvent.setup();
    renderView(client);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));

    await waitFor(() => expect(screen.getByText(/Preparing safety review…/)).toBeVisible());
    expect(screen.getByText(/Preparing safety review…/)).toHaveTextContent('1 item');
    const cta = screen.getByRole('button', { name: 'Clean Permanently' });
    expect(cta).toBeDisabled();
    expect(cta).toHaveTextContent('Delete 1 file permanently · 100 B');

    await act(async () => {
      resolveExplain({ itemId: 'cache-item-1', title: 'Cache entry', description: 'Cached data', risk: 'Safe' });
      await explainPromise;
    });

    await waitFor(() => expect(cta).toBeEnabled());
    expect(screen.queryByText(/Preparing safety review…/)).not.toBeInTheDocument();

    await user.click(cta);
    await waitFor(() => expect(executeClean).toHaveBeenCalledTimes(1));
  });
});
