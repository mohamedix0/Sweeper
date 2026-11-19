import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AnalyzerView } from './components/AnalyzerView';
import type { Settings, StartAnalyzerResult, SweeperClient } from './api/client';

function renderView(input: { client: SweeperClient; settings: Settings }) {
  return render(<AnalyzerView client={input.client} settings={input.settings} onOpenLog={() => {}} />);
}

function withDefaults(input: any) {
  let tier: any = 'Tier1';
  if (input.tier !== undefined) tier = input.tier;
  let targetId: any = 't1';
  if (input.targetId !== undefined) targetId = input.targetId;
  let risk: any = 'Safe';
  if (input.risk !== undefined) risk = input.risk;
  let bytes = '100';
  if (input.bytes !== undefined) bytes = input.bytes;
  return { ...input, isDir: Boolean(input.isDir), sizeBytes: bytes, itemCount: '1', risk, description: 'desc', reason: null, tier, targetId, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: input.selectable !== false, children: input.children ?? [] };
}

function node(input: any) {
  return withDefaults(input);
}

function dedupeResult(): StartAnalyzerResult {
  const safeParent = { ...node({ id: 'safe-parent', name: 'Temp', path: 'C:\\Temp', bytes: '300', isDir: true, targetId: 't-temp' }), itemCount: '3', children: [node({ id: 'safe-child-a', name: 'a.tmp', path: 'C:\\Temp\\a.tmp', targetId: 't-temp' }), node({ id: 'safe-child-b', name: 'b.tmp', path: 'C:\\Temp\\b.tmp', bytes: '200', targetId: 't-temp' })] };
  const unknownSel = { ...node({ id: 'unknown-sel', name: 'Mystery', path: 'C:\\Mystery', bytes: '50', risk: 'Unknown', tier: null, targetId: null, isDir: true }), itemCount: '1', children: [node({ id: 'unknown-child', name: 'x.bin', path: 'C:\\Mystery\\x.bin', bytes: '50', risk: 'Unknown', tier: null, targetId: null })] };
  const caution = node({ id: 'caution-node', name: 'Logs', path: 'C:\\Logs', bytes: '150', risk: 'Caution', tier: 'Tier2', targetId: 't-logs' });
  return { sessionId: 'an-dedupe', rootPath: 'C:\\', totalBytes: '500', totalItems: '5', scannedDirs: '3', scannedFiles: '4', cancelled: false, warnings: [], root: { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '500', itemCount: '5', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [safeParent, unknownSel, caution] } };
}

function explainForDedupe(input: { itemId: string }) {
  if (input.itemId.includes('unknown')) return { itemId: input.itemId, title: input.itemId, description: 'desc', risk: 'Unknown' as const };
  if (input.itemId.includes('caution')) return { itemId: input.itemId, title: input.itemId, description: 'desc', risk: 'Caution' as const };
  return { itemId: input.itemId, title: input.itemId, description: 'desc', risk: 'Safe' as const };
}

function dedupeBytes(input: { itemIds: string[] }) {
  if (input.itemIds.includes('safe-parent')) return '300';
  return '0';
}

async function runDedupeCase() {
  const previewClean = vi.fn(async (_sid: any, itemIds: any) => {
    const bytes = dedupeBytes({ itemIds });
    return { previewToken: 'tok', breakdown: [{ targetId: 't-temp', targetName: 'Temp', selectedCount: String(itemIds.length), selectedBytes: bytes }], totalSelectedCount: String(itemIds.length), totalSelectedBytes: bytes };
  });
  const client = { startAnalyzer: async () => dedupeResult(), explainItem: async (itemId: any) => explainForDedupe({ itemId: String(itemId) }), previewClean } as unknown as SweeperClient;
  const user = userEvent.setup();
  renderView({ client, settings: { allowDeleteUnknown: true } as Settings });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  await user.click(screen.getByRole('button', { name: 'Unknown (1)' }));
  expect(screen.getByLabelText('Select analyzer node C:\\Mystery')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: /^All \(/ }));
  await user.click(screen.getByRole('button', { name: 'Select Safe' }));
  expect(screen.getByLabelText('Select analyzer node C:\\Temp')).toBeChecked();
  const tempRow = screen.getByLabelText('Select analyzer node C:\\Temp').closest('[role="treeitem"]') as HTMLElement;
  await user.click(within(tempRow).getByRole('button', { name: /Expand Temp/i }));
  expect(await screen.findByLabelText('Select analyzer node C:\\Temp\\a.tmp')).not.toBeChecked();
  const reviewBtn = screen.getByRole('button', { name: 'Review & Clean' });
  await user.click(reviewBtn);
  await waitFor(() => expect(previewClean).toHaveBeenCalled());
  expect(previewClean.mock.calls[0][1]).toEqual(['safe-parent']);
}

function assertRects() {
  const rects = Array.from(document.querySelectorAll('svg.alloc-bar rect.alloc-segment'));
  expect(rects.length).toBeGreaterThan(0);
  for (const r of rects) {
    const x = Number(r.getAttribute('x') || '0');
    const w = Number(r.getAttribute('width') || '0');
    expect(x + w <= 1000.001).toBe(true);
  }
}

async function runBlockingCase() {
  const user = userEvent.setup();
  let resolveClean!: (v: any) => void;
  const cleanPromise = new Promise((r) => { resolveClean = r; });
  const children = Array.from({ length: 100 }, (_, i) => node({ id: `n-${i}`, name: `f-${i}.tmp`, path: `C:\\Bulk\\f-${i}.tmp`, bytes: '10', targetId: 't' }));
  const analyzerResult = { sessionId: 'an-ui', rootPath: 'C:\\', totalBytes: '1000', totalItems: '100', scannedDirs: '1', scannedFiles: '100', cancelled: false, warnings: [], root: { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '1000', itemCount: '100', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children } } as StartAnalyzerResult;
  const client = { startAnalyzer: async () => analyzerResult, explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }), previewClean: async () => ({ previewToken: 'tok-ui', breakdown: [{ targetId: 't', targetName: 'Bulk', selectedCount: '1', selectedBytes: '10' }], totalSelectedCount: '1', totalSelectedBytes: '10' }), executeClean: () => cleanPromise } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: false } as Settings });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  assertRects();
  const firstCheckbox = screen.getByLabelText('Select analyzer node C:\\Bulk\\f-0.tmp') as HTMLInputElement;
  await user.click(firstCheckbox);
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await screen.findByRole('button', { name: /Delete .* permanently/ });
  await user.click(screen.getByRole('button', { name: /Delete .* permanently/ }));
  const row = firstCheckbox.closest('[role="treeitem"]') as HTMLElement;
  await user.click(row);
  fireEvent.keyDown(firstCheckbox, { key: 'Enter', code: 'Enter', bubbles: true });
  expect(firstCheckbox).toBeChecked();
  await act(async () => { resolveClean({ bytesFreed: '10', deletedCount: '1', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }); await cleanPromise; });
}

async function runLateExplainCase() {
  const user = userEvent.setup();
  const deferred = new Map<string, { resolve: (v: any) => void }>();
  const analyzerResult = { sessionId: 'an-late', rootPath: 'C:\\', totalBytes: '200', totalItems: '2', scannedDirs: '1', scannedFiles: '2', cancelled: false, warnings: [], root: { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '200', itemCount: '2', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [node({ id: 'a', name: 'SafeA', path: 'C:\\SafeA' }), node({ id: 'b', name: 'SafeB', path: 'C:\\SafeB' })] } } as StartAnalyzerResult;
  const client = { startAnalyzer: async () => analyzerResult, explainItem: (itemId: any) => new Promise<any>((resolve) => { deferred.set(String(itemId), { resolve }); }) } as unknown as SweeperClient;
  renderView({ client, settings: { allowDeleteUnknown: false } as Settings });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  await user.click(await screen.findByLabelText('Select analyzer node C:\\SafeA'));
  await user.click(await screen.findByLabelText('Select analyzer node C:\\SafeB'));
  deferred.get('b')?.resolve({ itemId: 'b', title: 'SafeB', description: 'descB', risk: 'Safe' });
  await waitFor(() => expect(within(screen.getByLabelText('Analyzer inspector')).getAllByText('SafeB').length).toBeGreaterThan(0));
  deferred.get('a')?.resolve({ itemId: 'a', title: 'SafeA', description: 'descA', risk: 'Safe' });
  await waitFor(() => {
    const inspector = screen.getByLabelText('Analyzer inspector');
    expect(within(inspector).getAllByText('SafeB').length).toBeGreaterThan(0);
    expect(within(inspector).queryByText('SafeA')).not.toBeInTheDocument();
  });
}

function ancestorChildBulkResult(): StartAnalyzerResult {
  const child = node({ id: 'child-a', name: 'a.tmp', path: 'C:\\Temp\\a.tmp', targetId: 't-temp' });
  const parent = { ...node({ id: 'parent', name: 'Temp', path: 'C:\\Temp', bytes: '100', isDir: true, targetId: 't-temp' }), itemCount: '1', children: [child] };
  return { sessionId: 'an-bulk', rootPath: 'C:\\', totalBytes: '100', totalItems: '1', scannedDirs: '1', scannedFiles: '1', cancelled: false, warnings: [], root: { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [parent] } };
}

async function runAncestorBulkCase() {
  const previewClean = vi.fn(async (_sid: any, itemIds: any) => ({ previewToken: 'tok', breakdown: [{ targetId: 't-temp', targetName: 'Temp', selectedCount: String(itemIds.length), selectedBytes: '100' }], totalSelectedCount: String(itemIds.length), totalSelectedBytes: '100' }));
  const client = { startAnalyzer: async () => ancestorChildBulkResult(), explainItem: async (itemId: any) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }), previewClean } as unknown as SweeperClient;
  const user = userEvent.setup();
  renderView({ client, settings: { allowDeleteUnknown: false } as Settings });
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  await screen.findByRole('heading', { name: 'Analyzer Tree' });
  const parentRow = screen.getByLabelText('Select analyzer node C:\\Temp').closest('[role="treeitem"]') as HTMLElement;
  await user.click(within(parentRow).getByRole('button', { name: /Expand Temp/i }));
  expect(await screen.findByLabelText('Select analyzer node C:\\Temp\\a.tmp')).toBeInTheDocument();
  const master = screen.getByRole('checkbox', { name: /Select all visible selectable nodes|Deselect all visible selectable nodes/i });
  await user.click(master);
  await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
  await waitFor(() => expect(previewClean).toHaveBeenCalled());
  expect(previewClean.mock.calls[0][1]).toEqual(['parent']);
}

describe('AnalyzerView selection behavior', () => {
  it('AnalyzerView keeps selectable Unknown under Unknown and Select Safe dedupes non-overlapping safe roots', async () => {
    await runDedupeCase();
  });

  it('bulk select of visible ancestor and child does not double-select or drift from preview scope', async () => {
    await runAncestorBulkCase();
  });

  it('AnalyzerView blocks selection/drill while cleaning, ignores bubbled row keydown, and keeps treemap within viewBox for 100 equal children', async () => {
    await runBlockingCase();
  });

  it('AnalyzerView ignores late explainItem response after focusing another node', async () => {
    await runLateExplainCase();
  });
});