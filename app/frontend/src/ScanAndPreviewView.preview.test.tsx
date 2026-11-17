import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Profiler } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { ScanAndPreviewView } from './components/ScanAndPreviewView';
import type { ScanResults, SweeperClient } from './api/client';

type PreviewCall = { itemIds: string[]; resolve: (v: any) => void; promise: Promise<any> };

function scanResults(input: { sessionId: string }): ScanResults {
  return {
    sessionId: input.sessionId, startedUnixSecs: '1710000000', minAgeHours: 24,
    targets: [{ targetId: 't-safe', targetName: 'User Temp', tier: 'Tier1', risk: 'Safe', resolvedPaths: ['C:\\Temp'], scanned: true, statusReason: null, totalBytes: '200', eligibleBytes: '200', fileCount: '2', eligibleCount: '2', skippedCount: '0', items: [
      { id: 'i1', targetId: 't-safe', path: 'C:\\Temp\\a.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
      { id: 'i2', targetId: 't-safe', path: 'C:\\Temp\\b.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
    ] }],
  } as ScanResults;
}

function explain(input: { itemId: unknown }) {
  const itemId = String(input.itemId);
  return { itemId, title: itemId, description: 'desc', risk: 'Safe' as const };
}

function renderView(input: { client: SweeperClient }) {
  return render(<ScanAndPreviewView client={input.client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />);
}

async function openScan(input: { client: SweeperClient }) {
  const user = userEvent.setup();
  renderView(input);
  await user.click(screen.getByRole('button', { name: 'Start Scan' }));
  await screen.findByRole('heading', { name: 'Inspector' });
  return user;
}

async function selectBoth(input: { user: Awaited<ReturnType<typeof openScan>> }) {
  const cb1 = screen.getByLabelText('Select C:\\Temp\\a.tmp') as HTMLInputElement;
  const cb2 = screen.getByLabelText('Select C:\\Temp\\b.tmp') as HTMLInputElement;
  if (!cb1.checked) await input.user.click(cb1);
  if (!cb2.checked) await input.user.click(cb2);
  return { cb1, cb2 };
}

async function runDiscardCase() {
  let resolvePreview!: (value: any) => void;
  const previewPromise = new Promise<any>((resolve) => { resolvePreview = resolve; });
  let previewCalls = 0;
  const client = { startScan: async () => ({ sessionId: 'scan-1' }), getScanResults: async () => scanResults({ sessionId: 'scan-1' }), explainItem: async (itemId: any) => explain({ itemId }), previewClean: () => { previewCalls += 1; return previewPromise; } } as unknown as SweeperClient;
  const user = await openScan({ client });
  const boxes = await selectBoth({ user });
  await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
  await waitFor(() => expect(previewCalls).toBe(1));
  await user.click(boxes.cb2);
  resolvePreview({ sessionId: 'scan-1', previewToken: 'tok-stale', issuedAtUnixSecs: '1710000000', expiresAtUnixSecs: '1710000600', totalBytes: '200', totalItems: '2', requiresConfirmation: false, requiresElevation: false, perTarget: [] });
  await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeInTheDocument());
  expect(screen.queryByRole('button', { name: 'Clean Permanently' })).not.toBeInTheDocument();
  expect(screen.queryByText('Preview Breakdown')).not.toBeInTheDocument();
}

async function runClearsLoadingCase() {
  const calls: PreviewCall[] = [];
  const client = {
    startScan: async () => ({ sessionId: 'scan-preview-loading' }), getScanResults: async () => scanResults({ sessionId: 'scan-preview-loading' }), explainItem: async (itemId: any) => explain({ itemId }),
    previewClean: (_sid: any, itemIds: any) => {
      let resolve!: (v: any) => void;
      const promise = new Promise<any>((r) => { resolve = r; });
      calls.push({ itemIds, resolve, promise });
      return promise;
    },
  } as unknown as SweeperClient;
  const user = await openScan({ client });
  const boxes = await selectBoth({ user });
  await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
  await waitFor(() => expect(calls.length).toBe(1));
  expect(calls[0].itemIds).toEqual(['i1', 'i2']);
  await user.click(boxes.cb2);
  await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeEnabled());
  expect(screen.queryByRole('button', { name: 'Generating Preview…' })).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
  await waitFor(() => expect(calls.length).toBe(2));
  expect(calls[1].itemIds).toEqual(['i1']);
  await act(async () => {
    calls[0].resolve({ sessionId: 'scan-preview-loading', previewToken: 'tok-stale', breakdown: [{ targetId: 't-safe', targetName: 'User Temp', selectedCount: '2', selectedBytes: '200' }], totalSelectedCount: '2', totalSelectedBytes: '200' });
    calls[1].resolve({ sessionId: 'scan-preview-loading', previewToken: 'tok-new', breakdown: [{ targetId: 't-safe', targetName: 'User Temp', selectedCount: '1', selectedBytes: '100' }], totalSelectedCount: '1', totalSelectedBytes: '100' });
    await Promise.all([calls[0].promise, calls[1].promise]);
  });
  await waitFor(() => expect(screen.getByText('Preview Breakdown')).toBeInTheDocument());
  const breakdown = screen.getByText('Preview Breakdown').closest('.preview-breakdown') as HTMLElement;
  expect(within(breakdown).getByText(/1 item/i)).toBeInTheDocument();
  expect(within(breakdown).getAllByText(/100 B/).length).toBeGreaterThan(0);
  expect(within(breakdown).queryByText(/2 items/i)).not.toBeInTheDocument();
  expect(within(breakdown).queryByText(/200 B/)).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled();
}

describe('ScanAndPreviewView preview stale handling', () => {
  it('ScanAndPreviewView discards stale previewClean response when selection changes while pending', async () => {
    await runDiscardCase();
  });

  it('ScanAndPreviewView clears previewLoading when selection changes during in-flight previewClean', async () => {
    await runClearsLoadingCase();
  });
});

const CALLOUT_GROUP = 'Elevated risk in this selection';

function elevatedScanResults(sessionId: string): ScanResults {
  return {
    sessionId, startedUnixSecs: '1710000000', minAgeHours: 24,
    targets: [{ targetId: 't-caution', targetName: 'Browser Cache', tier: 'Tier1', risk: 'Caution', resolvedPaths: ['C:\\Cache'], scanned: true, statusReason: null, totalBytes: '100', eligibleBytes: '100', fileCount: '1', eligibleCount: '1', skippedCount: '0', items: [
      { id: 'i-elevated', targetId: 't-caution', path: 'C:\\Cache\\entry.tmp', size: '100', mtimeUnixSecs: '1700000000', ageEligible: true },
    ] }],
  } as ScanResults;
}

function previewClient(input: { sessionId: string; explainItem: (itemId: any) => Promise<any> }): SweeperClient {
  return {
    startScan: async () => ({ sessionId: input.sessionId }),
    getScanResults: async () => elevatedScanResults(input.sessionId),
    explainItem: input.explainItem,
    previewClean: async () => ({
      sessionId: input.sessionId, previewToken: 'tok', issuedAtUnixSecs: '1710000000', expiresAtUnixSecs: '1710000600',
      totalBytes: '100', totalItems: '1', requiresConfirmation: false, requiresElevation: false, perTarget: [],
      breakdown: [{ targetId: 't-caution', targetName: 'Browser Cache', selectedCount: '1', selectedBytes: '100' }],
      totalSelectedCount: '1', totalSelectedBytes: '100',
    }),
  } as unknown as SweeperClient;
}

async function selectElevatedItem(user: Awaited<ReturnType<typeof openScan>>) {
  const cb = screen.getByLabelText('Select C:\\Cache\\entry.tmp') as HTMLInputElement;
  if (!cb.checked) await user.click(cb);
}

/**
 * Profiler.onRender runs during the commit phase, after DOM mutations but
 * before passive effects. That captures the very first render where the Clean
 * button exists, before the explanation requests are even started.
 */
function renderViewWithProbe(input: { client: SweeperClient; record: (disabled: boolean | null) => void }) {
  return render(
    <Profiler id="view" onRender={() => {
      const button = screen.queryByRole('button', { name: 'Clean Permanently' }) as HTMLButtonElement | null;
      input.record(button ? button.disabled : null);
    }}>
      <ScanAndPreviewView client={input.client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />
    </Profiler>,
  );
}

describe('ScanAndPreviewView callout explanation gating', () => {
  it('ScanAndPreviewView disables Clean on the first Preview render before explanations resolve', async () => {
    let resolveExplain!: (value: any) => void;
    const explainPromise = new Promise<any>((resolve) => { resolveExplain = resolve; });
    const client = previewClient({ sessionId: 'scan-first-preview', explainItem: () => explainPromise });
    const samples: Array<boolean | null> = [];
    const user = userEvent.setup();
    renderViewWithProbe({ client, record: (disabled) => samples.push(disabled) });
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    await selectElevatedItem(user);
    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));

    await waitFor(() => expect(samples.some((value) => value !== null)).toBe(true));
    expect(samples.find((value) => value !== null)).toBe(true);
    expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeDisabled();

    await act(async () => {
      resolveExplain({ itemId: 'i-elevated', title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
      await explainPromise;
    });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled());
  });

  it('ScanAndPreviewView keeps Clean disabled until a delayed explanation resolves, then shows the callout', async () => {
    let calloutMode = false;
    let resolveCallout!: (value: any) => void;
    let calloutPromise: Promise<any> | null = null;
    const client = previewClient({
      sessionId: 'scan-delayed-explain',
      explainItem: (itemId: any) => {
        if (!calloutMode) return Promise.resolve({ itemId, title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
        if (!calloutPromise) calloutPromise = new Promise((resolve) => { resolveCallout = resolve; });
        return calloutPromise;
      },
    });
    const user = await openScan({ client });
    await selectElevatedItem(user);
    calloutMode = true;
    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(calloutPromise).not.toBeNull());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeDisabled());
    expect(screen.queryByRole('group', { name: CALLOUT_GROUP })).not.toBeInTheDocument();

    await act(async () => {
      resolveCallout({ itemId: 'i-elevated', title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
      await calloutPromise!;
    });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled());
    const group = screen.getByRole('group', { name: CALLOUT_GROUP });
    expect(within(group).getByText('Cache entry')).toBeInTheDocument();
  });

  it('ScanAndPreviewView keeps a fallback callout and reports a rejected explanation before re-enabling Clean', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      let calloutMode = false;
      let rejectCallout!: (reason?: any) => void;
      let calloutPromise: Promise<any> | null = null;
      const client = previewClient({
        sessionId: 'scan-rejected-explain',
        explainItem: (itemId: any) => {
          if (!calloutMode) return Promise.resolve({ itemId, title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
          if (!calloutPromise) calloutPromise = new Promise((_resolve, reject) => { rejectCallout = reject; });
          return calloutPromise;
        },
      });
      const user = await openScan({ client });
      await selectElevatedItem(user);
      calloutMode = true;
      await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
      await waitFor(() => expect(calloutPromise).not.toBeNull());
      await waitFor(() => expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeDisabled());
      expect(screen.queryByRole('group', { name: CALLOUT_GROUP })).not.toBeInTheDocument();

      await act(async () => {
        rejectCallout(new Error('explainItem failed'));
        await calloutPromise!.catch(() => {});
      });
      const group = await screen.findByRole('group', { name: CALLOUT_GROUP });
      expect(within(group).getByText('C:\\Cache\\entry.tmp')).toBeInTheDocument();
      expect(within(group).getByText('Caution')).toBeInTheDocument();
      expect(within(group).getByText(/Explanation unavailable/)).toBeInTheDocument();
      expect(screen.getByText(/Explanation unavailable for 1 selected item/)).toBeInTheDocument();
      expect(warnSpy).toHaveBeenCalled();
      await waitFor(() => expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled());
    } finally {
      warnSpy.mockRestore();
    }
  });
});