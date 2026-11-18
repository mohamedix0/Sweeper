import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { AnalyzerView } from './components/AnalyzerView';
import type { StartAnalyzerResult, SweeperClient } from './api/client';

type PreviewCall = { itemIds: string[]; resolve: (v: any) => void; promise: Promise<any> };

function result(input: { sessionId: string }): StartAnalyzerResult {
  return {
    sessionId: input.sessionId, rootPath: 'C:\\', totalBytes: '200', totalItems: '2', scannedDirs: '1', scannedFiles: '2', cancelled: false, warnings: [],
    root: { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '200', itemCount: '2', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [
      { id: 'n1', name: 'SafeA', path: 'C:\\SafeA', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
      { id: 'n2', name: 'SafeB', path: 'C:\\SafeB', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
    ] },
  };
}

function explain(input: { itemId: unknown }) {
  const itemId = String(input.itemId);
  return { itemId, title: itemId, description: 'desc', risk: 'Safe' as const };
}

async function openAnalyzer(input: { client: SweeperClient }) {
  const user = userEvent.setup();
  render(<AnalyzerView client={input.client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  return user;
}

async function selectBoth(input: { user: Awaited<ReturnType<typeof openAnalyzer>> }) {
  const cb1 = await screen.findByLabelText('Select analyzer node C:\\SafeA');
  const cb2 = await screen.findByLabelText('Select analyzer node C:\\SafeB');
  await input.user.click(cb1);
  await input.user.click(cb2);
  return { cb1, cb2 };
}

async function runDiscardCase() {
  let resolvePreview!: (value: any) => void;
  const previewPromise = new Promise<any>((resolve) => { resolvePreview = resolve; });
  let previewCalls = 0;
  const client = { startAnalyzer: async () => result({ sessionId: 'an-1' }), explainItem: async (itemId: any) => explain({ itemId }), previewClean: () => { previewCalls += 1; return previewPromise; } } as unknown as SweeperClient;
  const user = await openAnalyzer({ client });
  const boxes = await selectBoth({ user });
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await waitFor(() => expect(previewCalls).toBe(1));
  await user.click(boxes.cb2);
  resolvePreview({ sessionId: 'an-1', previewToken: 'tok-stale-an', issuedAtUnixSecs: '1710000000', expiresAtUnixSecs: '1710000600', totalBytes: '200', totalItems: '2', requiresConfirmation: false, requiresElevation: false, perTarget: [] });
  await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeInTheDocument());
  expect(screen.queryByRole('button', { name: /Delete .* permanently/ })).not.toBeInTheDocument();
  expect(screen.queryByText('Preview Breakdown')).not.toBeInTheDocument();
}

async function runClearsLoadingCase() {
  const calls: PreviewCall[] = [];
  const client = {
    startAnalyzer: async () => result({ sessionId: 'an-preview-loading' }),
    explainItem: async (itemId: any) => explain({ itemId }),
    previewClean: (_sid: any, itemIds: any) => {
      let resolve!: (v: any) => void;
      const promise = new Promise<any>((r) => { resolve = r; });
      calls.push({ itemIds, resolve, promise });
      return promise;
    },
  } as unknown as SweeperClient;
  const user = await openAnalyzer({ client });
  const boxes = await selectBoth({ user });
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await waitFor(() => expect(calls.length).toBe(1));
  expect(calls[0].itemIds).toEqual(['n1', 'n2']);
  await user.click(boxes.cb2);
  await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeEnabled());
  expect(screen.queryByRole('button', { name: 'Generating Preview…' })).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await waitFor(() => expect(calls.length).toBe(2));
  expect(calls[1].itemIds).toEqual(['n1']);
  await act(async () => {
    calls[0].resolve({ sessionId: 'an-preview-loading', previewToken: 'tok-stale-an', breakdown: [{ targetId: 't1', targetName: 'SafeA/SafeB', selectedCount: '2', selectedBytes: '200' }], totalSelectedCount: '2', totalSelectedBytes: '200' });
    calls[1].resolve({ sessionId: 'an-preview-loading', previewToken: 'tok-new-an', breakdown: [{ targetId: 't1', targetName: 'SafeA', selectedCount: '1', selectedBytes: '100' }], totalSelectedCount: '1', totalSelectedBytes: '100' });
    await Promise.all([calls[0].promise, calls[1].promise]);
  });
  await waitFor(() => expect(screen.getByText('Preview Breakdown')).toBeInTheDocument());
  const breakdown = screen.getByText('Preview Breakdown').closest('.preview-breakdown') as HTMLElement;
  expect(breakdown).toHaveTextContent(/1 item/i);
  expect(breakdown).toHaveTextContent(/100 B/);
  expect(breakdown).not.toHaveTextContent(/2 items/i);
  expect(breakdown).not.toHaveTextContent(/200 B/);
  expect(screen.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled();
}

describe('AnalyzerView preview stale handling', () => {
  it('AnalyzerView discards stale previewClean response when selection changes while pending', async () => {
    await runDiscardCase();
  });

  it('AnalyzerView clears previewLoading when selection changes during in-flight previewClean', async () => {
    await runClearsLoadingCase();
  });
});