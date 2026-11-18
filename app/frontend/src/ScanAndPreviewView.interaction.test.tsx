import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import type { ScanResults, Settings, SweeperClient } from './api/client';

function renderView(input: { client: SweeperClient; settings?: Settings | null }) {
  return render(<ScanAndPreviewView client={input.client} settings={input.settings ?? ({ allowDeleteUnknown: false } as Settings)} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
}

function threeItemResults(input: { sessionId: string }): ScanResults {
  return { sessionId: input.sessionId, startedUnixSecs: '1710000000', minAgeHours: 48, targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '300', eligibleBytes: '300', fileCount: '3', eligibleCount: '3', skippedCount: '0', items: [
    { id: 'a', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
    { id: 'b', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
    { id: 'c', targetId: 't-safe', path: 'C:\\Temp\\c.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
  ] }] } as ScanResults;
}

async function runStaleExplainCase() {
  const user = userEvent.setup();
  const deferredByItem = new Map<string, Array<{ resolve: (v: any) => void; promise: Promise<any> }>>();
  let scanCount = 0;
  let allowAutoInspect = true;
  const scanResults1 = threeItemResults({ sessionId: 'scan-1' });
  const scanResults2 = { ...scanResults1, sessionId: 'scan-2' };
  function defer(itemId: string) {
    let resolve!: (v: any) => void;
    const promise = new Promise<any>((r) => { resolve = r; });
    const list = deferredByItem.get(itemId) ?? [];
    list.push({ resolve, promise });
    deferredByItem.set(itemId, list);
    return promise;
  }
  function nextSession() {
    if (scanCount === 0) return 'scan-1';
    return 'scan-2';
  }
  function nextResults() {
    if (scanCount === 0) { scanCount += 1; return scanResults1; }
    scanCount += 1;
    return scanResults2;
  }
  const client = {
    startScan: async () => ({ sessionId: nextSession() }),
    getScanResults: async () => nextResults(),
    explainItem: (itemId: any) => {
      if (allowAutoInspect) { allowAutoInspect = false; return Promise.resolve({ itemId, title: `${String(itemId)} initial`, description: 'init', risk: 'Safe' as const }); }
      return defer(String(itemId));
    },
  } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  await waitFor(() => expect(within(screen.getByLabelText('Item inspector')).getByText('a.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument());
  await user.click(screen.getByLabelText('Select C:\\Temp\\a.tmp'));
  await user.click(screen.getByLabelText('Select C:\\Temp\\b.tmp'));
  const aDeferred = deferredByItem.get('a')?.[0];
  const bDeferred = deferredByItem.get('b')?.[0];
  await act(async () => { bDeferred!.resolve({ itemId: 'b', title: 'Item B latest', description: 'desc B', risk: 'Safe' }); await bDeferred!.promise; });
  await waitFor(() => expect(within(screen.getByLabelText('Item inspector')).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument());
  await act(async () => { aDeferred!.resolve({ itemId: 'a', title: 'Item A stale', description: 'desc A', risk: 'Safe' }); await aDeferred!.promise; });
  await waitFor(() => expect(within(screen.getByLabelText('Item inspector')).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument());
  allowAutoInspect = false;
  await user.click(screen.getByLabelText('Select C:\\Temp\\c.tmp'));
  const cDeferred = deferredByItem.get('c')?.[0];
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  await act(async () => { cDeferred!.resolve({ itemId: 'c', title: 'Item C stale reset', description: 'stale', risk: 'Safe' }); await cDeferred!.promise; });
  await waitFor(() => expect(within(screen.getByLabelText('Item inspector')).queryByText('c.tmp', { selector: '.inspector-headline-name' })).not.toBeInTheDocument());
}

async function runKeydownCase() {
  const user = userEvent.setup();
  const scanResults = { sessionId: 'scan-keys', startedUnixSecs: '1710000000', minAgeHours: 0, targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [{ id: 'k1', targetId: 't-safe', path: 'C:\\Temp\\k.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }] }] } as ScanResults;
  const client = { startScan: async () => ({ sessionId: 'scan-keys' }), getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }) } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: false, minAgeHours: 0 } as Settings });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  await user.click(screen.getByRole('button', { name: 'Unselect all' }));
  const cb = screen.getByLabelText('Select C:\\Temp\\k.tmp') as HTMLInputElement;
  cb.focus();
  fireEvent.keyDown(cb, { key: 'Enter', code: 'Enter', bubbles: true });
  expect(cb).not.toBeChecked();
  fireEvent.click(cb);
  await waitFor(() => expect(cb).toBeChecked());
  expect(screen.queryByText(/< 24h/)).not.toBeInTheDocument();
}

async function runAutoInspectCase() {
  const user = userEvent.setup();
  let resolveAutoExplain!: (v: any) => void;
  const autoExplainPromise = new Promise<any>((resolve) => { resolveAutoExplain = resolve; });
  let explainCalls = 0;
  const scanResults = { sessionId: 'scan-pending-explain', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '200', eligibleBytes: '200', fileCount: '2', eligibleCount: '2', skippedCount: '0', items: [{ id: 'a', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }, { id: 'b', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }] }] } as ScanResults;
  const client = { startScan: async () => ({ sessionId: 'scan-pending-explain' }), getScanResults: async () => scanResults, explainItem: (itemId: any) => { explainCalls += 1; if (explainCalls === 1) return autoExplainPromise; return Promise.resolve({ itemId, title: `${String(itemId)} focused`, description: 'focused', risk: 'Safe' as const }); } } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  expect(screen.getByRole('button', { name: 'Start Scan' })).toBeEnabled();
  await user.click(screen.getByLabelText('Select C:\\Temp\\b.tmp'));
  await waitFor(() => expect(within(screen.getByLabelText('Item inspector')).getByText('b.tmp', { selector: '.inspector-headline-name' })).toBeInTheDocument());
  resolveAutoExplain({ itemId: 'a', title: 'a stale', description: 'stale auto', risk: 'Safe' });
  await act(async () => { await autoExplainPromise; });
  await waitFor(() => expect(within(screen.getByLabelText('Item inspector')).queryByText('a.tmp', { selector: '.inspector-headline-name' })).not.toBeInTheDocument());
}

async function runBlockWhileCleaningCase() {
  const user = userEvent.setup();
  let resolveClean!: (v: any) => void;
  const cleanPromise = new Promise<any>((resolve) => { resolveClean = resolve; });
  const scanResults = { sessionId: 'scan-cleaning-block', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [{ id: 'i1', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }] }] } as ScanResults;
  const client = { startScan: async () => ({ sessionId: 'scan-cleaning-block' }), getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }), previewClean: async () => ({ sessionId: 'scan-cleaning-block', previewToken: 'tok', issuedAtUnixSecs: '1', expiresAtUnixSecs: '2', totalBytes: '100', totalItems: '1', requiresConfirmation: false, requiresElevation: false, perTarget: [], breakdown: [] }), executeClean: () => cleanPromise } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  const row = screen.getByLabelText('Select C:\\Temp\\a.tmp').closest('[role="button"]') as HTMLElement;
  const cb = screen.getByLabelText('Select C:\\Temp\\a.tmp') as HTMLInputElement;
  if (!cb.checked) await user.click(cb);
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await user.click(await screen.findByRole('button', { name: /Delete .* permanently/ }));
  await waitFor(() => expect(cb).toBeDisabled());
  const before = cb.checked;
  await user.click(row);
  fireEvent.keyDown(row, { key: 'Enter', code: 'Enter', bubbles: true });
  expect(cb.checked).toBe(before);
  await act(async () => { resolveClean({ bytesFreed: '100', deletedCount: '1', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }); await cleanPromise; });
}

async function runRunScanBlockedWhileCleaningCase() {
  const user = userEvent.setup();
  const scanRef = { current: null as any };
  let resolveClean!: (v: any) => void;
  const cleanPromise = new Promise<any>((resolve) => { resolveClean = resolve; });
  const startScan = vi.fn(async () => ({ sessionId: 'scan-clean-guard' }));
  const scanResults = { sessionId: 'scan-clean-guard', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [{ id: 'i1', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }] }] } as ScanResults;
  const client = { startScan, getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }), previewClean: async () => ({ sessionId: 'scan-clean-guard', previewToken: 'tok', issuedAtUnixSecs: '1', expiresAtUnixSecs: '2', totalBytes: '100', totalItems: '1', requiresConfirmation: false, requiresElevation: false, perTarget: [], breakdown: [] }), executeClean: () => cleanPromise } as unknown as SweeperClient;
  render(<ScanAndPreviewView ref={scanRef} client={client} settings={{ allowDeleteUnknown: false } as Settings} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  const callsAfterScan = startScan.mock.calls.length;
  const cb = screen.getByLabelText('Select C:\\Temp\\a.tmp') as HTMLInputElement;
  if (!cb.checked) await user.click(cb);
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await user.click(await screen.findByRole('button', { name: /Delete .* permanently/ }));
  await waitFor(() => expect(cb).toBeDisabled());
  await act(async () => { await scanRef.current?.runScan(); });
  expect(startScan.mock.calls.length).toBe(callsAfterScan);
  await act(async () => { resolveClean({ bytesFreed: '100', deletedCount: '1', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }); await cleanPromise; });
}

async function runGroupBadgeAndIndeterminateCase() {
  const user = userEvent.setup();
  const caution = { targetId: 't-caution', targetName: 'Vendor Logs', tier: 'Tier2', risk: 'Caution', resolvedPaths: ['C:\\Temp\\VendorLogs'], scanned: true, statusReason: null, totalBytes: '300', eligibleBytes: '200', fileCount: '2', eligibleCount: '1', skippedCount: '1', items: [{ id: 'c-old', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\old.log', size: '200', mtimeUnixSecs: '1700000000', ageEligible: true }, { id: 'c-new', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\new.log', size: '100', mtimeUnixSecs: '1899999999', ageEligible: false }] } as any;
  const unknownLocked = { targetId: 'custom-path-1', targetName: 'Custom Path (Unknown)', tier: 'Tier2', risk: 'Unknown', resolvedPaths: ['C:\\Temp\\CustomCleanup'], scanned: true, statusReason: null, totalBytes: '50', eligibleBytes: '0', fileCount: '1', eligibleCount: '0', skippedCount: '1', items: [{ id: 'u-old', targetId: 'custom-path-1', path: 'C:\\Temp\\CustomCleanup\\old.bin', size: '50', mtimeUnixSecs: '1700000000', ageEligible: true }] } as any;
  const scanResults = { sessionId: 'scan-badges', startedUnixSecs: '1710000000', minAgeHours: 48, targets: [caution, unknownLocked] } as ScanResults;
  const client = { startScan: async () => ({ sessionId: 'scan-badges' }), getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }) } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: false, minAgeHours: 48 } as Settings });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  const cautionGroup = screen.getByLabelText('Inspect Vendor Logs').closest('article') as HTMLElement;
  await user.click(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\old.log'));
  expect(within(within(cautionGroup).getByLabelText('Inspect Vendor Logs')).getByLabelText('Status Caution')).toHaveTextContent(/Caution/i);
  const groupCb = within(cautionGroup).getByLabelText('Select all eligible items in Vendor Logs') as HTMLInputElement;
  expect(groupCb.checked).toBe(true);
  expect(groupCb.indeterminate).toBe(false);
  await user.click(within(cautionGroup).getByLabelText('Inspect Vendor Logs'));
  const inspector = screen.getByLabelText('Inspector');
  expect(within(inspector).queryByText(/^Eligible$/)).not.toBeInTheDocument();
  const customGroup = screen.getByLabelText('Inspect Custom Path (Unknown)').closest('article') as HTMLElement;
  expect(within(within(customGroup).getByLabelText('Inspect Custom Path (Unknown)')).getByLabelText('Status Skipped')).toHaveTextContent(/Skipped · Unverified/);
}

describe('ScanAndPreviewView interaction', () => {
  it('ScanAndPreviewView ignores stale explain responses and invalidates in-flight explain on scan reset', async () => { await runStaleExplainCase(); });
  it('ScanAndPreviewView does not hijack keydown from nested row controls and shows no hard-coded 24h at min age 0', async () => { await runKeydownCase(); });
  it('ScanAndPreviewView completes scan immediately while auto-inspect explainItem is pending and ignores stale auto-inspect after focus change', async () => { await runAutoInspectCase(); });
  it('ScanAndPreviewView blocks row activation while cleaning', async () => { await runBlockWhileCleaningCase(); });
  it('blocks runScan while a clean is in progress', async () => { await runRunScanBlockedWhileCleaningCase(); });
  it('keeps caution/unknown risk badges visible on selection and avoids indeterminate when all eligible selected', async () => { await runGroupBadgeAndIndeterminateCase(); });
});