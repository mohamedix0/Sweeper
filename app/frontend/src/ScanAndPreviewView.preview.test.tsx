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
  expect(screen.queryByRole('button', { name: /Delete .* permanently/ })).not.toBeInTheDocument();
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
  expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled();
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
      const button = screen.queryByRole('button', { name: /Delete .* permanently/ }) as HTMLButtonElement | null;
      if (!button) {
        input.record(null);
        return;
      }
      input.record(button.disabled);
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
    expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeDisabled();

    await act(async () => {
      resolveExplain({ itemId: 'i-elevated', title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
      await explainPromise;
    });
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());
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
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeDisabled());
    expect(screen.queryByRole('group', { name: CALLOUT_GROUP })).not.toBeInTheDocument();

    await act(async () => {
      resolveCallout({ itemId: 'i-elevated', title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
      await calloutPromise!;
    });
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());
    const group = screen.getByRole('group', { name: CALLOUT_GROUP });
    expect(within(group).getByText('C:\\Cache\\entry.tmp')).toBeInTheDocument();
  });

  it('ScanAndPreviewView re-gates Clean on a second Preview with the same selection while its explanation is delayed', async () => {
    let delayExplain = false;
    let resolveSecond!: (value: any) => void;
    let secondExplain: Promise<any> | null = null;
    const client = previewClient({
      sessionId: 'scan-second-preview',
      explainItem: (itemId: any) => {
        if (!delayExplain) return Promise.resolve({ itemId, title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
        if (!secondExplain) secondExplain = new Promise((resolve) => { resolveSecond = resolve; });
        return secondExplain;
      },
    });
    const user = await openScan({ client });
    await selectElevatedItem(user);

    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());

    // Unselect all and reopen the Preview with the same selection. The resolved
    // explanations from the first Preview must not carry over to the second.
    await user.click(screen.getByRole('button', { name: 'Unselect all' }));
    await selectElevatedItem(user);
    delayExplain = true;
    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));
    await waitFor(() => expect(secondExplain).not.toBeNull());
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeDisabled());
    expect(screen.queryByRole('group', { name: CALLOUT_GROUP })).not.toBeInTheDocument();

    await act(async () => {
      resolveSecond({ itemId: 'i-elevated', title: 'Cache entry', description: 'Cached data', risk: 'Caution' });
      await secondExplain!;
    });
    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());
    expect(screen.getByRole('group', { name: CALLOUT_GROUP })).toBeInTheDocument();
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
      await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeDisabled());
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
      await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());
    } finally {
      warnSpy.mockRestore();
    }
  });

  it('ScanAndPreviewView keeps a Safe-only Preview callout-free when an explanation is rejected, while still showing the failure notice', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const client = {
        startScan: async () => ({ sessionId: 'scan-safe-rejected-explain' }),
        getScanResults: async () => scanResults({ sessionId: 'scan-safe-rejected-explain' }),
        explainItem: () => Promise.reject(new Error('explainItem failed')),
        previewClean: async () => ({
          sessionId: 'scan-safe-rejected-explain', previewToken: 'tok', issuedAtUnixSecs: '1710000000', expiresAtUnixSecs: '1710000600',
          totalBytes: '200', totalItems: '2', requiresConfirmation: false, requiresElevation: false, perTarget: [],
          breakdown: [{ targetId: 't-safe', targetName: 'User Temp', selectedCount: '2', selectedBytes: '200' }],
          totalSelectedCount: '2', totalSelectedBytes: '200',
        }),
      } as unknown as SweeperClient;
      const user = await openScan({ client });
      await selectBoth({ user });
      await user.click(screen.getByRole('button', { name: /Review & Clean/ }));

      // The rejected explanation resolves (Clean is re-enabled) but the known
      // Safe Risk is preserved, so no elevated-Risk callout is invented.
      await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled());
      expect(screen.queryByRole('group', { name: CALLOUT_GROUP })).not.toBeInTheDocument();
      expect(screen.queryByText(/Unknown risk:/)).not.toBeInTheDocument();

      // The explanation-failure notice is reported separately and stays visible.
      expect(screen.getByText(/Explanation unavailable for 2 selected item/)).toBeInTheDocument();
      expect(warnSpy).toHaveBeenCalled();
    } finally {
      warnSpy.mockRestore();
    }
  });
});

describe('ScanAndPreviewView large-selection risk coverage', () => {
  const TARGET_COUNT = 20;
  const ITEMS_PER_TARGET = 5;

  function manyCautionResults(sessionId: string): ScanResults {
    const targets = Array.from({ length: TARGET_COUNT }, (_, t) => {
      const targetId = `caution-${t}`;
      return {
        targetId, targetName: `Caution target ${t}`, tier: 'Tier2', risk: 'Caution',
        resolvedPaths: [`C:\\Users\\Alice\\Temp\\${t}`], scanned: true, statusReason: null,
        totalBytes: '500', eligibleBytes: '500', fileCount: String(ITEMS_PER_TARGET),
        eligibleCount: String(ITEMS_PER_TARGET), skippedCount: '0',
        items: Array.from({ length: ITEMS_PER_TARGET }, (_, i) => ({
          id: `${targetId}::${i + 1}`, targetId,
          path: `C:\\Users\\Alice\\Temp\\${t}\\item-${i + 1}`,
          size: '100', mtimeUnixSecs: '1700000000', ageEligible: true,
        })),
      };
    });
    return { sessionId, startedUnixSecs: '1710000000', minAgeHours: 24, targets } as ScanResults;
  }

  it('shows a warning for every elevated-risk item beyond the explanation cap before Clean enables', async () => {
    let explainCalls = 0;
    const sessionId = 'scan-large-caution';
    const client = {
      startScan: async () => ({ sessionId }),
      getScanResults: async () => manyCautionResults(sessionId),
      explainItem: async (itemId: any) => {
        explainCalls += 1;
        return { itemId, title: String(itemId), description: 'Knowledge Base entry', risk: 'Caution' as const };
      },
      previewClean: async () => ({
        sessionId, previewToken: 'tok', issuedAtUnixSecs: '1710000000', expiresAtUnixSecs: '1710000600',
        totalBytes: '10000', totalItems: '100', requiresConfirmation: false, requiresElevation: false, perTarget: [],
        breakdown: [], totalSelectedCount: '100', totalSelectedBytes: '10000',
      }),
    } as unknown as SweeperClient;

    const results = manyCautionResults(sessionId);
    const expectedIdentities = new Set(results.targets.flatMap((t) => t.items.map((i) => i.path)));
    const enabledCommits: string[][] = [];
    const user = userEvent.setup();
    render(
      <Profiler id="risk-coverage" onRender={() => {
        const cta = screen.queryByRole('button', { name: /Delete .* permanently/ }) as HTMLButtonElement | null;
        if (!cta) return;
        if (cta.disabled) return;
        const group = screen.queryByRole('group', { name: CALLOUT_GROUP });
        const notes = group ? within(group).queryAllByRole('note') : [];
        // An enabled commit with missing warnings is a safety regression.
        enabledCommits.push(notes.map((note) => (note.getAttribute('aria-label') ?? '').replace(/^Caution risk: /, '')));
      }}>
        <ScanAndPreviewView client={client} settings={{ allowDeleteUnknown: false } as any} settingsVersion={0} elevationStatus={{ isElevated: false }} elevationError={null} onOpenLog={() => {}} />
      </Profiler>,
    );
    await user.click(screen.getByRole('button', { name: 'Start Scan' }));
    await screen.findByRole('heading', { name: 'Inspector' });
    explainCalls = 0; // discard the scan-time auto-inspect explanation
    for (let t = 0; t < TARGET_COUNT; t += 1) {
      await user.click(screen.getByLabelText(`Select all eligible items in Caution target ${t}`));
    }
    await user.click(screen.getByRole('button', { name: /Review & Clean/ }));

    await waitFor(() => expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled(), { timeout: 10_000 });
    const group = screen.getByRole('group', { name: CALLOUT_GROUP });
    expect(within(group).getAllByRole('note')).toHaveLength(TARGET_COUNT * ITEMS_PER_TARGET);
    expect(explainCalls).toBeLessThanOrEqual(16);

    // Every commit where Clean was enabled already had complete warning coverage.
    expect(enabledCommits.length).toBeGreaterThan(0);
    for (const identities of enabledCommits) {
      expect(identities).toHaveLength(TARGET_COUNT * ITEMS_PER_TARGET);
    }
    const finalIdentities = enabledCommits[enabledCommits.length - 1];
    expect(new Set(finalIdentities)).toEqual(expectedIdentities);
    // Target 19 is beyond the 16-task explanation cap: its item still warns.
    expect(finalIdentities).toContain('C:\\Users\\Alice\\Temp\\19\\item-5');
  }, 20_000);
});