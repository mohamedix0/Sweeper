import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { ScanAndPreviewView, type ScanAndPreviewViewHandle } from './components/ScanAndPreviewView';
import type { ScanProgressEvent, ScanResults, Settings, SweeperClient, TargetScanResultDto } from './api/client';

function renderView(input: { client: SweeperClient; settings?: Settings | null; ref?: any }) {
  return render(<ScanAndPreviewView ref={input.ref} client={input.client} settings={input.settings ?? ({ allowDeleteUnknown: false } as Settings)} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
}

async function runProgressCase() {
  const user = userEvent.setup();
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const scanResults = { sessionId: 's1', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [] } as ScanResults;
  const client = {
    startScan: async (_tiers: any, onProgress?: (e: ScanProgressEvent) => void) => {
      onProgress?.({ kind: 'started', session_id: 's1' });
      onProgress?.({ kind: 'progress', session_id: 's1', completed_targets: 1, total_targets: 3, current_target_id: 't1', current_target_name: 'User Temp', scanned_items: '12', scanned_bytes: '1048576' });
      await gate;
      onProgress?.({ kind: 'completed', session_id: 's1' });
      return { sessionId: 's1' };
    },
    getScanResults: async () => scanResults,
    explainItem: async (itemId: any) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' as const }),
  } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  const notice = await screen.findByRole('status');
  expect(within(notice).getByText(/1 \/ 3 Targets/)).toBeInTheDocument();
  release();
  await waitFor(() => expect(screen.queryByText(/1 \/ 3 Targets/)).not.toBeInTheDocument());
}

function riskTargets(): TargetScanResultDto[] {
  const caution = { targetId: 't-caution', targetName: 'Vendor Logs', tier: 'Tier2', risk: 'Caution', resolvedPaths: ['C:\\Temp\\VendorLogs'], scanned: true, statusReason: null, totalBytes: '300', eligibleBytes: '200', fileCount: '2', eligibleCount: '1', skippedCount: '1', items: [{ id: 'c-old', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\old.log', size: '200', mtimeUnixSecs: '1700000000', ageEligible: true }, { id: 'c-new', targetId: 't-caution', path: 'C:\\Temp\\VendorLogs\\new.log', size: '100', mtimeUnixSecs: '1899999999', ageEligible: false }] } as TargetScanResultDto;
  const unknown = { targetId: 'custom-path-1', targetName: 'Custom Path (Unknown)', tier: 'Tier2', risk: 'Unknown', resolvedPaths: ['C:\\Temp\\CustomCleanup'], scanned: true, statusReason: null, totalBytes: '50', eligibleBytes: '50', fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [{ id: 'u-old', targetId: 'custom-path-1', path: 'C:\\Temp\\CustomCleanup\\old.bin', size: '50', mtimeUnixSecs: '1700000000', ageEligible: true }] } as TargetScanResultDto;
  return [caution, unknown];
}

async function runRiskCase() {
  const user = userEvent.setup();
  const scanResults = { sessionId: 'scan-risk', startedUnixSecs: '1710000000', minAgeHours: 48, targets: riskTargets() } as ScanResults;
  const client = { startScan: async () => ({ sessionId: 'scan-risk' }), getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }) } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: true, minAgeHours: 24 } as Settings });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  const cautionGroup = screen.getByLabelText('Inspect Vendor Logs').closest('article') as HTMLElement;
  expect(within(within(cautionGroup).getByLabelText('Inspect Vendor Logs')).getByLabelText('Status Caution')).toHaveTextContent(/Caution/i);
  expect(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\new.log').closest('[role="button"]')).toHaveTextContent('< 48h');
  await user.click(screen.getByLabelText('Select C:\\Temp\\VendorLogs\\old.log'));
  expect((await screen.findAllByText(/> 48h/)).length).toBeGreaterThan(0);
}

async function runOverlapCase() {
  const scanRef = createRef<ScanAndPreviewViewHandle>();
  let resolveScan!: (v: { sessionId: string }) => void;
  const scanPromise = new Promise<{ sessionId: string }>((resolve) => { resolveScan = resolve; });
  const startScan = vi.fn(() => scanPromise);
  const client = { startScan, getScanResults: vi.fn(async () => ({ sessionId: 'scan-overlap', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [] } as ScanResults)), explainItem: vi.fn(async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const })), previewClean: vi.fn() } as unknown as SweeperClient;
  renderView({ client, ref: scanRef });
  const firstRun = scanRef.current?.runScan();
  await waitFor(() => expect(startScan).toHaveBeenCalledTimes(1));
  void scanRef.current?.runScan();
  await waitFor(() => expect(startScan).toHaveBeenCalledTimes(1));
  await act(async () => { resolveScan({ sessionId: 'scan-overlap' }); await firstRun; });
}

async function runGuardCase() {
  let resolveStart!: (v: any) => void;
  const startPromise = new Promise<any>((resolve) => { resolveStart = resolve; });
  let startCalls = 0;
  const scanResults = { sessionId: 'scan-overlap', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [] } as ScanResults;
  const client = { startScan: () => { startCalls += 1; return startPromise; }, getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }) } as unknown as SweeperClient;
  const ref = createRef<ScanAndPreviewViewHandle>();
  renderView({ client, ref });
  const p1 = ref.current!.runScan();
  const p2 = ref.current!.runScan();
  expect(startCalls).toBe(1);
  await act(async () => { resolveStart({ sessionId: 'scan-overlap' }); await Promise.all([p1, p2]); });
  expect(screen.getByRole('button', { name: 'Start Scan' })).toBeEnabled();
}

async function runTier3IncludedCase() {
  const user = userEvent.setup();
  const startScan = vi.fn(async () => ({ sessionId: 's1' }));
  const client = {
    startScan,
    getScanResults: async () => ({ sessionId: 's1', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [] } as ScanResults),
    explainItem: async (itemId: any) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' as const }),
  } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: false, enableTier3: true } as Settings });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await waitFor(() => expect(startScan).toHaveBeenCalled());
  expect(startScan).toHaveBeenCalledWith(expect.arrayContaining(['Tier1', 'Tier2', 'Tier3']), expect.any(Function));
}

async function runEmptyScopeBlockedCase() {
  const user = userEvent.setup();
  const startScan = vi.fn(async () => ({ sessionId: 's1' }));
  const client = { startScan, getScanResults: async () => ({ sessionId: 's1', startedUnixSecs: '1', minAgeHours: 24, targets: [] } as ScanResults), explainItem: async (itemId: any) => ({ itemId, title: 'x', description: 'd', risk: 'Safe' as const }) } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: false, enableTier3: false } as Settings });
  await user.click(screen.getByLabelText('Dev Caches'));
  await user.click(screen.getByLabelText('Temp Files'));
  const startBtn = screen.getByRole('button', { name: 'Start Scan' });
  expect(startBtn).toBeDisabled();
  await user.click(startBtn);
  expect(startScan).not.toHaveBeenCalled();
}

async function runScopeNoAutoRescanCase() {
  const user = userEvent.setup();
  const startScan = vi.fn(async () => ({ sessionId: 's1' }));
  const scanResults = { sessionId: 's1', startedUnixSecs: '1710000000', minAgeHours: 24, targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [{ id: 'i1', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true }] }] } as ScanResults;
  const client = { startScan, getScanResults: async () => scanResults, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'd', risk: 'Safe' as const }) } as unknown as SweeperClient;
  renderView({ client });
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  const callsBefore = startScan.mock.calls.length;
  await user.click(screen.getByLabelText('Dev Caches'));
  await waitFor(() => expect(screen.getByLabelText('Select C:\\Temp\\a.tmp')).toBeInTheDocument());
  expect(startScan.mock.calls.length).toBe(callsBefore);
  expect(screen.getByLabelText('Select C:\\Temp\\a.tmp')).toBeInTheDocument();
}

describe('ScanAndPreviewView behavior', () => {
  it('renders incremental scan progress updates (Bug 2)', async () => { await runProgressCase(); });
  it('ScanAndPreviewView keeps eligible Caution/Unknown group statuses and uses dynamic min age labels', async () => { await runRiskCase(); });
  it('ScanAndPreviewView does not start a second concurrent startScan while scan is active (Scan Again overlap)', async () => { await runOverlapCase(); });
  it('guards overlapping imperative runScan when a duplicate trigger fires while scan is active', async () => { await runGuardCase(); });
  it('includes Tier3 when enableTier3 is true', async () => { await runTier3IncludedCase(); });
  it('blocks starting scan when no tiers are active', async () => { await runEmptyScopeBlockedCase(); });
  it('preserves scan results and selection when scope changes until explicit Rescan', async () => { await runScopeNoAutoRescanCase(); });
});