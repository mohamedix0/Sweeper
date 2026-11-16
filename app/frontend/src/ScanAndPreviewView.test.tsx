import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { ScanAndPreviewView, type ScanAndPreviewViewHandle } from './components/ScanAndPreviewView';
import { mockController } from './api/mock';
import type { ContractError, ScanItemDto, ScanProgressEvent, ScanResults, Settings, SweeperClient, TargetScanResultDto } from './api/client';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('enforces typed confirmation for Custom Path/Unknown scan items and requires exact DELETE', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    const expandCustomPathBtn = screen.queryByRole('button', { name: /Expand Custom Path/i });
    if (expandCustomPathBtn) await user.click(expandCustomPathBtn);

    const unknownCb = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
    expect(unknownCb).toBeDisabled(); // gated by allow_delete_unknown

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    const allowUnknown = screen.getByLabelText('Allow deleting unknown folders (advanced)');
    await user.click(allowUnknown);
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    await screen.findByText(/Settings saved/i);

    await user.click(screen.getByRole('button', { name: 'Scan & Clean' }));
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    const expandCustomPathBtn2 = screen.queryByRole('button', { name: /Expand Custom Path/i });
    if (expandCustomPathBtn2) await user.click(expandCustomPathBtn2);

    const unknownCb2 = screen.getByLabelText('Select C:\\Users\\Alice\\CustomCleanup\\old.bin') as HTMLInputElement;
    expect(unknownCb2).toBeEnabled();
    await user.click(unknownCb2);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Review & Clean' });
    expect(previewBtn).toBeDisabled();

    await user.type(screen.getByLabelText('Typed confirmation'), 'delete');
    expect(previewBtn).toBeDisabled();
    await user.clear(screen.getByLabelText('Typed confirmation'));
    await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
    expect(previewBtn).toBeEnabled();
    await user.click(previewBtn);
    await screen.findByText('Preview Breakdown');
  });

  it('handles expired/invalidated session errors cleanly', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    await user.click(screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp'));
    mockController.advanceTimeMs(15 * 60 * 1000 + 1000);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/expired|invalid/i);
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Inspector' })).not.toBeInTheDocument());
  });

  it('supports scan bulk selection, target-group selection, and status filters', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    const selectAllSafe = screen.getByRole('button', { name: 'Select Safe' });
    expect(selectAllSafe).toBeEnabled();
    await user.click(selectAllSafe);

    const safeEligible = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
    expect(safeEligible).toBeChecked();
    expect(screen.getAllByRole('button', { name: 'Clear' }).length).toBeGreaterThan(0);

    expect(screen.getAllByLabelText('Status Skipped').length).toBeGreaterThan(0);
    await user.click(screen.getByRole('button', { name: /^Eligible \(/ }));
    await waitFor(() => expect(screen.queryAllByLabelText('Status Skipped')).toHaveLength(0));

    await user.click(screen.getAllByRole('button', { name: 'Clear' })[0]);
    await waitFor(() => expect(safeEligible.checked).toBe(false));

    await user.click(screen.getByRole('button', { name: /^All \(/ }));
    const userTempGroup = screen.getByLabelText('Select all eligible items in User Temp') as HTMLInputElement;
    await user.click(userTempGroup);
    expect(safeEligible).toBeChecked();
  });

  it('renders incremental scan progress updates (Bug 2)', async () => {
    const { user } = userEvent.setup ? { user: userEvent.setup() } : { user: userEvent };
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const scanResults: ScanResults = {
      sessionId: 's1',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [],
    };
    const client = {
      startScan: async (_tiers: unknown, onProgress?: (e: ScanProgressEvent) => void) => {
        onProgress?.({ kind: 'started', session_id: 's1' });
        onProgress?.({
          kind: 'progress',
          session_id: 's1',
          completed_targets: 1,
          total_targets: 3,
          current_target_id: 't1',
          current_target_name: 'User Temp',
          scanned_items: '12',
          scanned_bytes: '1048576',
        });
        await gate;
        onProgress?.({ kind: 'completed', session_id: 's1' });
        return { sessionId: 's1' };
      },
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={null} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    const notice = await screen.findByRole('status');
    function expectScanProgressNoticeDetails() {
      expect(notice).toHaveClass('scan-progress-notice');
      const targets = within(notice).getByText(/1 \/ 3 Targets/);
      expect(targets).toBeInTheDocument();
      expect(within(notice).getAllByText(/User Temp/).length).toBeGreaterThan(0);
      const items = within(notice).getByText(/12 items/);
      expect(items).toBeInTheDocument();
      expect(notice.querySelector('.scan-progress-meter')).not.toBeNull();
    }
    expectScanProgressNoticeDetails();
    release();
    await waitFor(() => expect(screen.queryByText(/1 \/ 3 Targets/)).not.toBeInTheDocument());
  });

  it('ScanAndPreviewView discards stale previewClean response when selection changes while pending', async () => {
    const user = userEvent.setup();
    let resolvePreview!: (value: any) => void;
    const previewPromise = new Promise<any>((resolve) => { resolvePreview = resolve; });
    const scanResults: ScanResults = {
      sessionId: 'scan-1',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [
        {
          targetId: 'user-temp',
          targetName: 'User Temp',
          tier: 'Tier1',
          risk: 'Safe',
          statusReason: null,
          totalBytes: '200',
          eligibleBytes: '200',
          itemCount: '2',
          scanned: true,
          resolvedPaths: ['C:\\Users\\Alice\\AppData\\Local\\Temp'],
          items: [
            { id: 'i1', targetId: 'user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
            { id: 'i2', targetId: 'user-temp', path: 'C:\\Users\\Alice\\AppData\\Local\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          ],
        } as any,
      ],
    };
    let previewCalls = 0;
    const client = {
      startScan: async () => ({ sessionId: 'scan-1' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: () => { previewCalls += 1; return previewPromise; },
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    const cb1 = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp') as HTMLInputElement;
    const cb2 = screen.getByLabelText('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\b.tmp') as HTMLInputElement;
    if (!cb1.checked) await user.click(cb1);
    if (!cb2.checked) await user.click(cb2);

    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(previewCalls).toBe(1));

    await user.click(cb2); // change selection while preview pending
    resolvePreview({
      sessionId: 'scan-1',
      previewToken: 'tok-stale',
      issuedAtUnixSecs: '1710000000',
      expiresAtUnixSecs: '1710000600',
      totalBytes: '200',
      totalItems: '2',
      requiresConfirmation: false,
      requiresElevation: false,
      perTarget: [],
    });

    await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Clean Permanently' })).not.toBeInTheDocument();
    expect(screen.queryByText('Preview Breakdown')).not.toBeInTheDocument();
  });

  it('ScanAndPreviewView keeps eligible Caution/Unknown group statuses and uses dynamic min age labels', async () => {
    const user = userEvent.setup();
    const cautionTarget: TargetScanResultDto = {
      targetId: 't-caution',
      targetName: 'Vendor Logs',
      tier: 'Tier2',
      risk: 'Caution',
      resolvedPaths: ['C:\\Temp\\VendorLogs'],
      scanned: true,
      statusReason: null,
      totalBytes: '300',
      eligibleBytes: '200',
      fileCount: '2',
      eligibleCount: '1',
      skippedCount: '1',
      items: [
        { id: 'c-old', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\old.log', size: '200', mtimeUnixSecs: '1700000000', ageEligible: true },
        { id: 'c-new', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\new.log', size: '100', mtimeUnixSecs: '1899999999', ageEligible: false },
      ],
    };
    const unknownTarget: TargetScanResultDto = {
      targetId: 'custom-path-1',
      targetName: 'Custom Path (Unknown)',
      tier: 'Tier2',
      risk: 'Unknown',
      resolvedPaths: ['C:\\Temp\\CustomCleanup'],
      scanned: true,
      statusReason: null,
      totalBytes: '50',
      eligibleBytes: '50',
      fileCount: '1',
      eligibleCount: '1',
      skippedCount: '0',
      items: [
        { id: 'u-old', targetId: 'custom-path-1', path: 'C:\\Temp\\CustomCleanup\\old.bin', size: '50', mtimeUnixSecs: '1700000000', ageEligible: true },
      ],
    };
    const scanResults: ScanResults = {
      sessionId: 'scan-risk',
      startedUnixSecs: '1710000000',
      minAgeHours: 48,
      targets: [cautionTarget, unknownTarget],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-risk' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(
      <ScanAndPreviewView
        client={client}
        settings={{ allowDeleteUnknown: true, minAgeHours: 24 } as Settings}
        settingsVersion={0}
        elevationStatus={{ isElevated: false }}
        elevationError={null}
        onOpenLog={() => {}}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    const cautionGroup = screen.getByLabelText('Inspect Vendor Logs').closest('article') as HTMLElement;
    const cautionGroupHeader = within(cautionGroup).getByLabelText('Inspect Vendor Logs');
    expect(within(cautionGroupHeader).getByLabelText('Status Caution')).toHaveTextContent(/1 Caution|Caution/i);
    expect(within(cautionGroupHeader).queryByLabelText('Status Safe')).not.toBeInTheDocument();

    const unknownGroup = screen.getByLabelText('Inspect Custom Path (Unknown)').closest('article') as HTMLElement;
    const unknownGroupHeader = within(unknownGroup).getByLabelText('Inspect Custom Path (Unknown)');
    expect(within(unknownGroupHeader).getByLabelText('Status Unknown')).toHaveTextContent(/Unknown/i);
    expect(within(unknownGroupHeader).queryByLabelText('Status Safe')).not.toBeInTheDocument();

    expect(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\new.log').closest('[role="button"]')).toHaveTextContent('< 48h');
    await user.click(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\old.log'));
    const over48 = await screen.findAllByText(/> 48h/);
    expect(over48.length).toBeGreaterThan(0);
    expect(screen.queryByText(/> 24h/)).not.toBeInTheDocument();
    expect(screen.queryByText(/< 24h/)).not.toBeInTheDocument();
  });

  it('ScanAndPreviewView ignores stale explain responses and invalidates in-flight explain on scan reset', async () => {
    const user = userEvent.setup();
    const deferredByItem = new Map<string, Array<{ resolve: (v: any) => void; promise: Promise<any> }>>();
    let scanCount = 0;
    let allowAutoInspect = true;
    const scanResults1: ScanResults = {
      sessionId: 'scan-1',
      startedUnixSecs: '1710000000',
      minAgeHours: 48,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '300', eligibleBytes: '300', fileCount: '3', eligibleCount: '3', skippedCount: '0',
        items: [
          { id: 'a', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          { id: 'b', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          { id: 'c', targetId: 't-safe', path: 'C:\\Temp\\c.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
        ],
      }],
    };
    const scanResults2: ScanResults = { ...scanResults1, sessionId: 'scan-2' };
    const deferExplain = (itemId: string) => {
      let resolve!: (v: any) => void;
      const promise = new Promise<any>((r) => { resolve = r; });
      const list = deferredByItem.get(itemId) ?? [];
      list.push({ resolve, promise });
      deferredByItem.set(itemId, list);
      return promise;
    };
    const client = {
      startScan: async () => ({ sessionId: scanCount === 0 ? 'scan-1' : 'scan-2' }),
      getScanResults: async () => (scanCount++ === 0 ? scanResults1 : scanResults2),
      explainItem: (itemId: string) => {
        if (allowAutoInspect) {
          allowAutoInspect = false;
          return Promise.resolve({ itemId, title: `${itemId} initial`, description: 'init', risk: 'Safe' as const });
        }
        return deferExplain(itemId);
      },
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as Settings} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    // Initial auto-inspect focuses item a and shows its details (auto-inspect may resolve after scan completion).
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('a.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
    });

    await user.click(screen.getByLabelText('Select C:\\Temp\\a.tmp')); // focus a (older pending explain)
    await user.click(screen.getByLabelText('Select C:\\Temp\\b.tmp')); // focus b (latest pending explain)
    const aDeferred = deferredByItem.get('a')?.[0];
    const bDeferred = deferredByItem.get('b')?.[0];
    expect(aDeferred).toBeTruthy();
    expect(bDeferred).toBeTruthy();

    await act(async () => {
      bDeferred!.resolve({ itemId: 'b', title: 'Item B latest', description: 'desc B', risk: 'Safe' });
      await bDeferred!.promise;
    });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
    });

    await act(async () => {
      aDeferred!.resolve({ itemId: 'a', title: 'Item A stale', description: 'desc A', risk: 'Safe' });
      await aDeferred!.promise;
    });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
      expect(within(inspector).queryByText('Select an item to see its purpose, risk, full path, and eligibility.')).not.toBeInTheDocument();
    });

    // Start a new pending explain for c, then rescan to invalidate it.
    allowAutoInspect = false; // next explain call after reset should be deferred (rescan auto-focus)
    await user.click(screen.getByLabelText('Select C:\\Temp\\c.tmp'));
    const cDeferred = deferredByItem.get('c')?.[0];
    expect(cDeferred).toBeTruthy();
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    await act(async () => {
      cDeferred!.resolve({ itemId: 'c', title: 'Item C stale reset', description: 'stale', risk: 'Safe' });
      await cDeferred!.promise;
    });
    // Rescan clears/invalidates in-flight explain; stale c response must not repopulate item c context.
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).queryByText('c.tmp', { selector: '.inspector-headline-name' })).not.toBeInTheDocument();
    });
  });

  it('ScanAndPreviewView does not hijack keydown from nested row controls and shows no hard-coded 24h at min age 0', async () => {
    const user = userEvent.setup();
    const scanResults: ScanResults = {
      sessionId: 'scan-keys',
      startedUnixSecs: '1710000000',
      minAgeHours: 0,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0',
        items: [{ id: 'k1', targetId: 't-safe', path: 'C:\\Temp\\k.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }],
      }],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-keys' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false, minAgeHours: 0 } as Settings} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });

    await user.click(screen.getByRole('button', { name: 'Clear' }));
    const cb = screen.getByLabelText('Select C:\\Temp\\k.tmp') as HTMLInputElement;
    cb.focus();
    expect(cb).not.toBeChecked();
    fireEvent.keyDown(cb, { key: 'Enter', code: 'Enter', bubbles: true });
    expect(cb).not.toBeChecked();
    fireEvent.click(cb); // realistic toggle via control interaction
    await waitFor(() => expect(cb).toBeChecked());
    fireEvent.keyDown(cb, { key: ' ', code: 'Space', bubbles: true });
    expect(cb).toBeChecked();
    expect(screen.queryByText(/< 24h/)).not.toBeInTheDocument();
    expect(screen.queryByText(/24h/)).not.toBeInTheDocument();
  });

  it('ScanAndPreviewView completes scan immediately while auto-inspect explainItem is pending and ignores stale auto-inspect after focus change', async () => {
    const user = userEvent.setup();
    let resolveAutoExplain!: (v: any) => void;
    const autoExplainPromise = new Promise<any>((resolve) => { resolveAutoExplain = resolve; });
    let explainCalls = 0;
    const scanResults: ScanResults = {
      sessionId: 'scan-pending-explain',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '200', eligibleBytes: '200', fileCount: '2', eligibleCount: '2', skippedCount: '0',
        items: [
          { id: 'a', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          { id: 'b', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
        ],
      }],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-pending-explain' }),
      getScanResults: async () => scanResults,
      explainItem: (itemId: string) => {
        explainCalls += 1;
        if (explainCalls === 1) return autoExplainPromise;
        return Promise.resolve({ itemId, title: `${itemId} focused`, description: 'focused', risk: 'Safe' as const });
      },
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as Settings} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));

    await screen.findByRole('heading', { name: 'Inspector' });
    expect(screen.getByRole('button', { name: 'Start Scan' })).toBeEnabled();
    expect(screen.queryByText(/Starting Scan|Scanning/i)).not.toBeInTheDocument();

    await user.click(screen.getByLabelText('Select C:\\Temp\\b.tmp'));
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
    });

    resolveAutoExplain({ itemId: 'a', title: 'a stale', description: 'stale auto', risk: 'Safe' });
    await act(async () => { await autoExplainPromise; });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Item inspector');
      expect(within(inspector).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument();
      expect(within(inspector).queryByText('a.tmp', { selector: '.inspector-headline-name' })).not.toBeInTheDocument();
    });
  });

  it('ScanAndPreviewView clears previewLoading when selection changes during in-flight previewClean', async () => {
    const user = userEvent.setup();
    const previewCalls: Array<{ itemIds: string[]; resolve: (v: any) => void; promise: Promise<any> }> = [];
    const scanResults: ScanResults = {
      sessionId: 'scan-preview-loading',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '200', eligibleBytes: '200', fileCount: '2', eligibleCount: '2', skippedCount: '0',
        items: [
          { id: 'i1', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
          { id: 'i2', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
        ],
      }],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-preview-loading' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: (_sid: string, itemIds: string[]) => {
        let resolve!: (v: any) => void;
        const promise = new Promise<any>((r) => { resolve = r; });
        previewCalls.push({ itemIds, resolve, promise });
        return promise;
      },
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    const cb1 = screen.getByLabelText('Select C:\\Temp\\a.tmp') as HTMLInputElement;
    const cb2 = screen.getByLabelText('Select C:\\Temp\\b.tmp') as HTMLInputElement;
    if (!cb1.checked) await user.click(cb1);
    if (!cb2.checked) await user.click(cb2);
    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(previewCalls.length).toBe(1));
    expect(previewCalls[0].itemIds).toEqual(['i1', 'i2']);

    await user.click(cb2);
    await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeEnabled());
    expect(screen.queryByRole('button', { name: 'Generating Preview…' })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(previewCalls.length).toBe(2));
    expect(previewCalls[1].itemIds).toEqual(['i1']);

    await act(async () => {
      previewCalls[0].resolve({ sessionId: 'scan-preview-loading', previewToken: 'tok-stale', breakdown: [{ targetId: 't-safe', targetName: 'User Temp', selectedCount: '2', selectedBytes: '200' }], totalSelectedCount: '2', totalSelectedBytes: '200' });
      previewCalls[1].resolve({ sessionId: 'scan-preview-loading', previewToken: 'tok-new', breakdown: [{ targetId: 't-safe', targetName: 'User Temp', selectedCount: '1', selectedBytes: '100' }], totalSelectedCount: '1', totalSelectedBytes: '100' });
      await Promise.all([previewCalls[0].promise, previewCalls[1].promise]);
    });

    await waitFor(() => expect(screen.getByText('Preview Breakdown')).toBeInTheDocument());
    const breakdown = screen.getByText('Preview Breakdown').closest('.preview-breakdown') as HTMLElement;
    function expectScanPreviewBreakdownOneItem() {
      expect(within(breakdown).getByText(/1 item/i)).toBeInTheDocument();
      const size = within(breakdown).getAllByText(/100 B/).length;
      expect(size).toBeGreaterThan(0);
      expect(within(breakdown).queryByText(/2 items/i)).not.toBeInTheDocument();
      const missing = within(breakdown).queryByText(/200 B/);
      expect(missing).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled();
    }
    expectScanPreviewBreakdownOneItem();
  });

  it('ScanAndPreviewView does not start a second concurrent startScan while scan is active (Scan Again overlap)', async () => {
    const scanRef = createRef<ScanAndPreviewViewHandle>();
    let resolveScan!: (v: { sessionId: string }) => void;
    const scanPromise = new Promise<{ sessionId: string }>((resolve) => { resolveScan = resolve; });
    const startScan = vi.fn(() => scanPromise);
    const client = {
      startScan,
      getScanResults: vi.fn(async () => ({ sessionId: 'scan-overlap', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [] } as ScanResults)),
      explainItem: vi.fn(async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const })),
      previewClean: vi.fn(),
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView ref={scanRef} client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);

    const firstRun = scanRef.current?.runScan();
    await waitFor(() => expect(startScan).toHaveBeenCalledTimes(1));

    void scanRef.current?.runScan();
    await waitFor(() => expect(startScan).toHaveBeenCalledTimes(1));

    await act(async () => {
      resolveScan({ sessionId: 'scan-overlap' });
      await firstRun;
    });
  });

  it('ScanAndPreviewView blocks row activation while cleaning', async () => {
    const user = userEvent.setup();
    let resolveClean!: (v: any) => void;
    const cleanPromise = new Promise<any>((resolve) => { resolveClean = resolve; });
    const scanResults: ScanResults = {
      sessionId: 'scan-cleaning-block',
      startedUnixSecs: '1710000000',
      minAgeHours: 24,
      targets: [{
        targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null,
        totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0',
        items: [{ id: 'i1', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }],
      }],
    };
    const client = {
      startScan: async () => ({ sessionId: 'scan-cleaning-block' }),
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: async () => ({ sessionId: 'scan-cleaning-block', previewToken: 'tok', issuedAtUnixSecs: '1', expiresAtUnixSecs: '2', totalBytes: '100', totalItems: '1', requiresConfirmation: false, requiresElevation: false, perTarget: [], breakdown: [] }),
      executeClean: () => cleanPromise,
    } as unknown as SweeperClient;

    render(<ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    const row = screen.getByLabelText('Select C:\\Temp\\a.tmp').closest('[role="button"]') as HTMLElement;
    const cb = screen.getByLabelText('Select C:\\Temp\\a.tmp') as HTMLInputElement;
    if (!cb.checked) await user.click(cb);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    const cleanBtn = await screen.findByRole('button', { name: 'Clean Permanently' });
    await user.click(cleanBtn);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Start Scan' })).toBeDisabled());
    await waitFor(() => expect(cb).toBeDisabled());
    const before = cb.checked;
    await user.click(row);
    fireEvent.keyDown(row, { key: 'Enter', code: 'Enter', bubbles: true });
    fireEvent.keyDown(row, { key: ' ', code: 'Space', bubbles: true });
    expect(cb.checked).toBe(before);
    await act(async () => { resolveClean({ bytesFreed: '100', deletedCount: '1', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }); await cleanPromise; });
  });

  it('guards overlapping imperative runScan when a duplicate trigger fires while scan is active', async () => {
    let resolveStart!: (v: any) => void;
    const startPromise = new Promise<any>((resolve) => { resolveStart = resolve; });
    let startCalls = 0;
    const scanResults: ScanResults = { sessionId: 'scan-overlap', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [] };
    const client = {
      startScan: () => { startCalls += 1; return startPromise; },
      getScanResults: async () => scanResults,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;
    const ref = createRef<ScanAndPreviewViewHandle>();

    render(<ScanAndPreviewView ref={ref} client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
    const p1 = ref.current!.runScan();
    const p2 = ref.current!.runScan();
    expect(startCalls).toBe(1);
    await act(async () => { resolveStart({ sessionId: 'scan-overlap' }); await Promise.all([p1, p2]); });
    expect(screen.getByRole('button', { name: 'Start Scan' })).toBeEnabled();
  });
});
