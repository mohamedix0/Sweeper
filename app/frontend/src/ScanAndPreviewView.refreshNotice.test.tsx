import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import type { ScanResults, SweeperClient } from './api/client';

const STALE_ITEM = 'C:\\Temp\\cache\\stale.tmp';

function staleResults(): ScanResults {
  return {
    sessionId: 'scan-refresh-1',
    startedUnixSecs: '1710000000',
    minAgeHours: 24,
    targets: [{
      targetId: 't-cache',
      targetName: 'Cache',
      tier: 'Tier1',
      risk: 'Safe',
      resolvedPaths: ['C:\\Temp\\cache'],
      scanned: true,
      statusReason: null,
      totalBytes: '100',
      eligibleBytes: '100',
      fileCount: '1',
      eligibleCount: '1',
      skippedCount: '0',
      items: [{ id: 'cache-item-1', targetId: 't-cache', path: STALE_ITEM, size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }],
    }],
  } as ScanResults;
}

function previewResult() {
  return {
    sessionId: 'scan-refresh-1',
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

describe('ScanAndPreviewView post-Clean refresh notice', () => {
  it('shows a non-blocking notice and drops stale results when the refresh fails', async () => {
    const getScanResults = vi.fn()
      .mockResolvedValueOnce(staleResults())
      .mockRejectedValueOnce(new Error('refresh failed'));
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
      startScan: async () => ({ sessionId: 'scan-refresh-1' }),
      getScanResults,
      explainItem: async (itemId: string) => ({ itemId, title: 'Cache entry', description: 'Cached data', risk: 'Safe' }),
      previewClean: async () => previewResult(),
      executeClean,
    } as unknown as SweeperClient;

    const user = userEvent.setup();
    renderView(client);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    expect(await screen.findByLabelText(`Select ${STALE_ITEM}`)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());
    await user.click(screen.getByRole('button', { name: /Delete .* permanently/ }));

    await waitFor(() => expect(executeClean).toHaveBeenCalledTimes(1));
    expect(getScanResults).toHaveBeenCalledTimes(2);
    expect(await screen.findByText(/could not be refreshed/i)).toBeVisible();
    expect(screen.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
    await waitFor(() => expect(screen.queryByLabelText(`Select ${STALE_ITEM}`)).not.toBeInTheDocument());
  });
});
