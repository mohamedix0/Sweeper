import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { createRef } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { AnalyzerView } from './components/AnalyzerView';
import { mockController } from './api/mock';
import type { AnalyzerProgressEvent, ContractError, Settings, StartAnalyzerResult, SweeperClient } from './api/client';

function setup() {
  return { user: userEvent.setup(), ...render(<App />) };
}

describe('Sweeper frontend', () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.dataset.theme = 'dark';
    mockController.reset();
  });

  it('Analyzer gates Unknown selection behind allow_unknown and requires typed confirmation for Unknown nodes', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const unknownPath = 'C:\\Users\\Alice\\AppData\\Local\\MysteryApp\\MysteryCache';
    let unknownCb = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    if (!unknownCb) {
      // Root is expanded by default; if virtualization/state timing hides children in tests, re-toggle root.
      const collapseRoot = screen.queryByRole('button', { name: /Collapse /i });
      if (collapseRoot) {
        await user.click(collapseRoot);
        await user.click(await screen.findByRole('button', { name: /Expand /i }));
      }
      unknownCb = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    }
    if (!unknownCb) {
      // Fallback: locate the gated Unknown node row by its status text.
      const statuses = await screen.findAllByText(/Unknown gated by Settings/i);
      const row = statuses[0]?.closest('[role="treeitem"]') as HTMLElement | null;
      expect(row).not.toBeNull();
      unknownCb = within(row!).getByRole('checkbox') as HTMLInputElement;
    }
    expect(unknownCb).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });
    await user.click(screen.getByLabelText('Allow deleting unknown folders (advanced)'));
    await user.click(screen.getAllByRole('button', { name: 'Save Settings' })[0]);
    await screen.findByText(/Settings saved/i);

    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    let unknownCb2 = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    if (!unknownCb2) {
      const collapseRoot = screen.queryByRole('button', { name: /Collapse /i });
      if (collapseRoot) {
        await user.click(collapseRoot);
        await user.click(await screen.findByRole('button', { name: /Expand /i }));
      }
      unknownCb2 = screen.queryByLabelText(`Select analyzer node ${unknownPath}`) as HTMLInputElement | null;
    }
    if (!unknownCb2) {
      // After allowing unknowns, select the first Unknown-selectable checkbox by finding a node whose status indicates typed confirmation required for Unknown/custom path.
      const candidates = screen.getAllByRole('checkbox', { name: /Select analyzer node /i }) as HTMLInputElement[];
      unknownCb2 = candidates.find((cb) => !cb.disabled) ?? null;
    }
    expect(unknownCb2).not.toBeNull();
    expect(unknownCb2!).toBeEnabled();
    await user.click(unknownCb2!);
    expect(await screen.findByLabelText('Typed confirmation')).toBeInTheDocument();
    const previewBtn = screen.getByRole('button', { name: 'Review & Clean' });
    expect(previewBtn).toBeDisabled();
    await user.type(screen.getByLabelText('Typed confirmation'), 'DELETE');
    expect(previewBtn).toBeEnabled();
  });

  it('marks non-selectable analyzer nodes (.vhdx / Protected / Link Point) correctly', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const all = screen.getAllByRole('checkbox') as HTMLInputElement[];
    const findBySuffix = (suffix: string) => all.find((n) => (n.getAttribute('aria-label') ?? '').includes(suffix));
    const vhdx = findBySuffix('dev-machine.vhdx');
    expect(vhdx).toBeTruthy();
    expect(vhdx!).toBeDisabled();
    expect(vhdx!.closest('[role="treeitem"]')).toHaveTextContent(/\.vhdx/i);

    const gitNode = findBySuffix('\\Dev\\repo\\.git');
    expect(gitNode).toBeTruthy();
    expect(gitNode!).toBeDisabled();
    expect(gitNode!.closest('[role="treeitem"]')).toHaveTextContent(/Protected/i);

    const linkNode = findBySuffix('\\Users\\Alice\\OneDriveLink');
    expect(linkNode).toBeTruthy();
    expect(linkNode!).toBeDisabled();
    expect(linkNode!.closest('[role="treeitem"]')).toHaveTextContent(/Link Point/i);
  });

  it('shows Analyzer sticky action bar Review & Clean with disabled/enabled states', async () => {
    const { user } = setup();
    await user.click(screen.getByRole('button', { name: 'Analyzer' }));
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const previewBtn = screen.getByRole('button', { name: 'Review & Clean' }) as HTMLButtonElement;
    expect(previewBtn).toBeDisabled();

    const safeNode = await screen.findByLabelText(/Select analyzer node .*AppData\\Local\\Temp$/);
    await user.click(safeNode);
    expect(previewBtn).toBeEnabled();
    expect(previewBtn.className).toMatch(/primary/);
  });

  it('renders analyzer long-path progress in bounded overflow-safe container (Bug 3a)', async () => {
    const { user } = userEvent.setup ? { user: userEvent.setup() } : { user: userEvent };
    const longPath = 'C:\\Users\\Alice\\AppData\\Local\\Temp\\sweeper-sandbox-FwSDTi\\target\\d0000\\d0001\\d0002\\d0003\\very-long-unbroken-directory-name-without-spaces-for-overflow-testing';
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const minimalRoot = {
      id: 'root',
      name: 'C:',
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
      children: [],
    };
    const client = {
      startAnalyzer: async (_root: string | null, onProgress?: (e: AnalyzerProgressEvent) => void): Promise<StartAnalyzerResult> => {
        onProgress?.({ sessionId: 'a1', scannedDirs: '1', scannedFiles: '0', scannedBytes: '0', currentPath: 'C:\\' });
        onProgress?.({ sessionId: 'a1', scannedDirs: '42', scannedFiles: '7', scannedBytes: '4096', currentPath: longPath });
        await gate;
        return {
          sessionId: 'a1',
          rootPath: 'C:\\',
          totalBytes: '0',
          totalItems: '0',
          scannedDirs: '42',
          scannedFiles: '7',
          cancelled: false,
          warnings: [],
          root: minimalRoot,
        };
      },
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={null} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    const notice = await screen.findByRole('status');
    expect(notice).toHaveClass('analyzer-progress-notice');
    expect(notice).toHaveClass('scanning-pulse');
    const pathEl = notice.querySelector('.analyzer-progress-path');
    expect(pathEl).not.toBeNull();
    expect(pathEl).toHaveAttribute('title', longPath);
    expect(pathEl?.textContent).toContain('sweeper-sandbox-FwSDTi');
    release();
    await waitFor(() => expect(screen.queryByText(/Analyzing…/)).not.toBeInTheDocument());
  });

  it('AnalyzerView discards stale previewClean response when selection changes while pending', async () => {
    const user = userEvent.setup();
    let resolvePreview!: (value: any) => void;
    const previewPromise = new Promise<any>((resolve) => { resolvePreview = resolve; });
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-1',
      rootPath: 'C:\\',
      totalBytes: '200',
      totalItems: '2',
      scannedDirs: '1',
      scannedFiles: '2',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root',
        name: 'C:',
        path: 'C:\\',
        isDir: true,
        sizeBytes: '200',
        itemCount: '2',
        risk: 'Safe',
        description: 'root',
        reason: null,
        tier: null,
        targetId: null,
        isLinkPoint: false,
        isCloudPlaceholder: false,
        displayOnly: false,
        selectable: false,
        children: [
          {
            id: 'n1',
            name: 'SafeA',
            path: 'C:\\SafeA',
            isDir: true,
            sizeBytes: '100',
            itemCount: '1',
            risk: 'Safe',
            description: 'safe',
            reason: null,
            tier: 'Tier1',
            targetId: 't1',
            isLinkPoint: false,
            isCloudPlaceholder: false,
            displayOnly: false,
            selectable: true,
            children: [],
          },
          {
            id: 'n2',
            name: 'SafeB',
            path: 'C:\\SafeB',
            isDir: true,
            sizeBytes: '100',
            itemCount: '1',
            risk: 'Safe',
            description: 'safe',
            reason: null,
            tier: 'Tier1',
            targetId: 't1',
            isLinkPoint: false,
            isCloudPlaceholder: false,
            displayOnly: false,
            selectable: true,
            children: [],
          },
        ],
      },
    };
    let previewCalls = 0;
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: () => { previewCalls += 1; return previewPromise; },
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const cb1 = await screen.findByLabelText('Select analyzer node C:\\SafeA') as HTMLInputElement;
    const cb2 = await screen.findByLabelText('Select analyzer node C:\\SafeB') as HTMLInputElement;
    await user.click(cb1);
    await user.click(cb2);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await waitFor(() => expect(previewCalls).toBe(1));

    await user.click(cb2); // change selection while preview pending
    resolvePreview({
      sessionId: 'an-1',
      previewToken: 'tok-stale-an',
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

  it('AnalyzerView escapes VHDX PowerShell path as single-quoted literal in render and copy', async () => {
    const user = userEvent.setup();
    let copied = '';
    const originalClipboard = navigator.clipboard;
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: async (text: string) => { copied = text; } },
    });
    try {
      const maliciousPath = "C:\\VMs\\evil'$(Start-Process calc).vhdx";
      const analyzerResult: StartAnalyzerResult = {
        sessionId: 'an-vhdx',
        rootPath: 'C:\\',
        totalBytes: '10',
        totalItems: '1',
        scannedDirs: '1',
        scannedFiles: '1',
        cancelled: false,
        warnings: [],
        root: {
          id: 'root',
          name: 'C:',
          path: 'C:\\',
          isDir: true,
          sizeBytes: '10',
          itemCount: '1',
          risk: 'Safe',
          description: 'root',
          reason: null,
          tier: null,
          targetId: null,
          isLinkPoint: false,
          isCloudPlaceholder: false,
          displayOnly: false,
          selectable: false,
          children: [
            {
              id: 'v1',
              name: "evil'$(Start-Process calc).vhdx",
              path: maliciousPath,
              isDir: false,
              sizeBytes: '10',
              itemCount: '1',
              risk: 'Caution',
              description: 'vhdx',
              reason: null,
              tier: null,
              targetId: null,
              isLinkPoint: false,
              isCloudPlaceholder: false,
              displayOnly: true,
              selectable: false,
              children: [],
            },
          ],
        },
      };
      const client = {
        startAnalyzer: async () => analyzerResult,
        explainItem: async (itemId: string) => ({
          itemId,
          title: itemId === 'v1' ? "evil'$(Start-Process calc).vhdx" : 'root',
          description: 'desc',
          risk: 'Caution' as const,
        }),
      } as unknown as SweeperClient;

      render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
      await user.click(screen.getByRole('button', { name: 'Analyze' }));
      await screen.findByRole('heading', { name: 'Analyzer Tree' });
      const vhdxRow = (await screen.findAllByText(/\.vhdx$/i))[0].closest('[role="treeitem"]') as HTMLElement;
      await user.click(vhdxRow);

      const expectedEscapedPath = "'C:\\VMs\\evil''$(Start-Process calc).vhdx'";
      const expectedCmd = `wsl --shutdown\nOptimize-VHD -Path ${expectedEscapedPath} -Mode Full`;
      expect(await screen.findByText(/Optimize-VHD -Path/)).toBeInTheDocument();
      const cmdNode = screen.getByText(/Optimize-VHD -Path/).closest('code');
      expect(cmdNode?.textContent).toBe(expectedCmd);
      expect(cmdNode?.textContent).not.toContain(`"${maliciousPath}"`);

      await user.click(screen.getByRole('button', { name: 'Copy Command' }));
      await waitFor(() => expect(copied).toBe(expectedCmd));
    } finally {
      Object.defineProperty(navigator, 'clipboard', { configurable: true, value: originalClipboard });
    }
  });

  it('AnalyzerView exposes Use Preset with a neutral root preset', async () => {
    const user = userEvent.setup();
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-preset',
      rootPath: 'C:\\Users',
      totalBytes: '0',
      totalItems: '0',
      scannedDirs: '0',
      scannedFiles: '0',
      cancelled: false,
      warnings: [],
      root: { id: 'root', name: 'C:\\Users', path: 'C:\\Users', isDir: true, sizeBytes: '0', itemCount: '0', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [] },
    };
    const startAnalyzer = vi.fn(async (path: string | null) => ({ ...analyzerResult, rootPath: path ?? analyzerResult.rootPath, root: { ...analyzerResult.root, name: path ?? analyzerResult.root.name, path: path ?? analyzerResult.root.path } }));
    const client = {
      startAnalyzer,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as Settings} onOpenLog={() => {}} />);
    const usePreset = screen.getByRole('button', { name: 'Use preset root path' });
    await user.click(usePreset);
    const rootInput = screen.getByLabelText('Analyzer root path') as HTMLInputElement;
    expect(['C:\\', 'C:\\Users', 'C:\\Windows\\Temp']).toContain(rootInput.value);
    expect(rootInput.value.toLowerCase()).not.toMatch(/\\users\\[^\\]/);
  });

  it('AnalyzerView keeps selectable Unknown under Unknown and Select Safe dedupes non-overlapping safe roots', async () => {
    const user = userEvent.setup();
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-dedupe',
      rootPath: 'C:\\',
      totalBytes: '500',
      totalItems: '5',
      scannedDirs: '3',
      scannedFiles: '4',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '500', itemCount: '5', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children: [
          {
            id: 'safe-parent', name: 'Temp', path: 'C:\\Temp', isDir: true, sizeBytes: '300', itemCount: '3', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
            children: [
              { id: 'safe-child-a', name: 'a.tmp', path: 'C:\\Temp\\a.tmp', isDir: false, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
              { id: 'safe-child-b', name: 'b.tmp', path: 'C:\\Temp\\b.tmp', isDir: false, sizeBytes: '200', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't-temp', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
            ],
          },
          {
            id: 'unknown-sel', name: 'Mystery', path: 'C:\\Mystery', isDir: true, sizeBytes: '50', itemCount: '1', risk: 'Unknown', description: 'unknown', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true,
            children: [
              { id: 'unknown-child', name: 'x.bin', path: 'C:\\Mystery\\x.bin', isDir: false, sizeBytes: '50', itemCount: '1', risk: 'Unknown', description: 'unknown', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
            ],
          },
          { id: 'caution-node', name: 'Logs', path: 'C:\\Logs', isDir: false, sizeBytes: '150', itemCount: '1', risk: 'Caution', description: 'caution', reason: null, tier: 'Tier2', targetId: 't-logs', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
        ],
      },
    };
    const previewClean = vi.fn(async (_sid: string, itemIds: string[]) => ({
      previewToken: 'tok',
      breakdown: [{ targetId: 't-temp', targetName: 'Temp', selectedCount: String(itemIds.length), selectedBytes: String(itemIds.includes('safe-parent') ? 300 : 0) }],
      totalSelectedCount: String(itemIds.length),
      totalSelectedBytes: String(itemIds.includes('safe-parent') ? 300 : 0),
    }));
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: itemId.includes('unknown') ? 'Unknown' as const : itemId.includes('caution') ? 'Caution' as const : 'Safe' as const }),
      previewClean,
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: true } as Settings} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    expect(screen.getByRole('button', { name: 'Unknown (1)' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Safe (1)' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Unknown (1)' }));
    expect(screen.getByLabelText('Select analyzer node C:\\Mystery')).toBeInTheDocument();
    expect(screen.queryByLabelText('Select analyzer node C:\\Temp')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /^All \(/ }));

    await user.click(screen.getByRole('button', { name: 'Select Safe' }));
    expect(screen.getByLabelText('Select analyzer node C:\\Temp')).toBeChecked();
    const tempRow = screen.getByLabelText('Select analyzer node C:\\Temp').closest('[role="treeitem"]') as HTMLElement;
    await user.click(within(tempRow).getByRole('button', { name: /Expand Temp/i }));
    expect(await screen.findByLabelText('Select analyzer node C:\\Temp\\a.tmp')).not.toBeChecked();
    expect(screen.getByLabelText('Select analyzer node C:\\Temp\\b.tmp')).not.toBeChecked();
    expect(screen.getByText(/1 item selected/i)).toBeInTheDocument();
    const reviewBtn = screen.getByRole('button', { name: 'Review & Clean' });
    expect(reviewBtn).toHaveTextContent('300 B');

    await user.click(reviewBtn);
    await waitFor(() => expect(previewClean).toHaveBeenCalled());
    expect(previewClean.mock.calls[0][1]).toEqual(['safe-parent']);
  });

  it('AnalyzerView blocks selection/drill while cleaning, ignores bubbled row keydown, and keeps treemap within viewBox for 100 equal children', async () => {
    const user = userEvent.setup();
    let resolveClean!: (v: any) => void;
    const cleanPromise = new Promise((r) => { resolveClean = r; });
    const children = Array.from({ length: 100 }, (_, i) => ({
      id: `n-${i}`, name: `f-${i}.tmp`, path: `C:\\Bulk\\f-${i}.tmp`, isDir: false, sizeBytes: '10', itemCount: '1', risk: 'Safe' as const,
      description: 'safe', reason: null, tier: 'Tier1' as const, targetId: 't', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [],
    }));
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-ui',
      rootPath: 'C:\\',
      totalBytes: '1000',
      totalItems: '100',
      scannedDirs: '1',
      scannedFiles: '100',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '1000', itemCount: '100', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children,
      },
    };
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: async () => ({
        previewToken: 'tok-ui',
        breakdown: [{ targetId: 't', targetName: 'Bulk', selectedCount: '1', selectedBytes: '10' }],
        totalSelectedCount: '1',
        totalSelectedBytes: '10',
      }),
      executeClean: () => cleanPromise,
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as Settings} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const rects = Array.from(document.querySelectorAll('svg.alloc-bar rect.alloc-segment'));
    expect(rects.length).toBeGreaterThan(0);
    for (const r of rects) {
      const x = Number(r.getAttribute('x') || '0');
      const w = Number(r.getAttribute('width') || '0');
      expect(x).toBeGreaterThanOrEqual(0);
      expect(w).toBeGreaterThanOrEqual(0);
      expect(x + w).toBeLessThanOrEqual(1000.001);
    }

    const firstCheckbox = screen.getByLabelText('Select analyzer node C:\\Bulk\\f-0.tmp') as HTMLInputElement;
    await user.click(firstCheckbox);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await screen.findByRole('button', { name: 'Clean Permanently' });
    await user.click(screen.getByRole('button', { name: 'Clean Permanently' }));
    expect(screen.getByText(/Cleaning in progress/i)).toBeInTheDocument();

    const row = firstCheckbox.closest('[role="treeitem"]') as HTMLElement;
    await user.click(row);
    expect(firstCheckbox).toBeChecked();
    firstCheckbox.focus();
    fireEvent.keyDown(firstCheckbox, { key: 'Enter', code: 'Enter', bubbles: true });
    expect(firstCheckbox).toBeChecked();
    const drillBtn = within(row).queryByRole('button', { name: /Drill into/i });
    if (drillBtn) expect(drillBtn).toBeDisabled();

    await act(async () => { resolveClean({ bytesFreed: '10', deletedCount: '1', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }); await cleanPromise; });
  });

  it('AnalyzerView ignores late explainItem response after focusing another node', async () => {
    const user = userEvent.setup();
    const deferred = new Map<string, { resolve: (v: any) => void }>();
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-late',
      rootPath: 'C:\\',
      totalBytes: '200',
      totalItems: '2',
      scannedDirs: '1',
      scannedFiles: '2',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '200', itemCount: '2', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children: [
          { id: 'a', name: 'SafeA', path: 'C:\\SafeA', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
          { id: 'b', name: 'SafeB', path: 'C:\\SafeB', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
        ],
      },
    };
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: (itemId: string) => new Promise<any>((resolve) => { deferred.set(itemId, { resolve }); }),
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });

    const cbA = await screen.findByLabelText('Select analyzer node C:\\SafeA');
    await user.click(cbA);
    const cbB = await screen.findByLabelText('Select analyzer node C:\\SafeB');
    await user.click(cbB);

    deferred.get('b')?.resolve({ itemId: 'b', title: 'SafeB', description: 'descB', risk: 'Safe' });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Analyzer inspector');
      expect(within(inspector).getAllByText('SafeB').length).toBeGreaterThan(0);
    });

    deferred.get('a')?.resolve({ itemId: 'a', title: 'SafeA', description: 'descA', risk: 'Safe' });
    await waitFor(() => {
      const inspector = screen.getByLabelText('Analyzer inspector');
      expect(within(inspector).getAllByText('SafeB').length).toBeGreaterThan(0);
      expect(within(inspector).queryByText('SafeA')).not.toBeInTheDocument();
    });
  });

  it('AnalyzerView clears previewLoading when selection changes during in-flight previewClean', async () => {
    const user = userEvent.setup();
    const previewCalls: Array<{ itemIds: string[]; resolve: (v: any) => void; promise: Promise<any> }> = [];
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-preview-loading',
      rootPath: 'C:\\',
      totalBytes: '200',
      totalItems: '2',
      scannedDirs: '1',
      scannedFiles: '2',
      cancelled: false,
      warnings: [],
      root: {
        id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '200', itemCount: '2', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false,
        children: [
          { id: 'n1', name: 'SafeA', path: 'C:\\SafeA', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
          { id: 'n2', name: 'SafeB', path: 'C:\\SafeB', isDir: true, sizeBytes: '100', itemCount: '1', risk: 'Safe', description: 'safe', reason: null, tier: 'Tier1', targetId: 't1', isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: true, children: [] },
        ],
      },
    };
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: itemId, description: 'desc', risk: 'Safe' as const }),
      previewClean: (_sid: string, itemIds: string[]) => {
        let resolve!: (v: any) => void;
        const promise = new Promise<any>((r) => { resolve = r; });
        previewCalls.push({ itemIds, resolve, promise });
        return promise;
      },
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    const cb1 = await screen.findByLabelText('Select analyzer node C:\\SafeA') as HTMLInputElement;
    const cb2 = await screen.findByLabelText('Select analyzer node C:\\SafeB') as HTMLInputElement;
    await user.click(cb1);
    await user.click(cb2);
    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await waitFor(() => expect(previewCalls.length).toBe(1));
    expect(previewCalls[0].itemIds).toEqual(['n1', 'n2']);

    await user.click(cb2);
    await waitFor(() => expect(screen.getByRole('button', { name: /Review & Clean/ })).toBeEnabled());
    expect(screen.queryByRole('button', { name: 'Generating Preview…' })).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Review & Clean' }));
    await waitFor(() => expect(previewCalls.length).toBe(2));
    expect(previewCalls[1].itemIds).toEqual(['n1']);

    await act(async () => {
      previewCalls[0].resolve({ sessionId: 'an-preview-loading', previewToken: 'tok-stale-an', breakdown: [{ targetId: 't1', targetName: 'SafeA/SafeB', selectedCount: '2', selectedBytes: '200' }], totalSelectedCount: '2', totalSelectedBytes: '200' });
      previewCalls[1].resolve({ sessionId: 'an-preview-loading', previewToken: 'tok-new-an', breakdown: [{ targetId: 't1', targetName: 'SafeA', selectedCount: '1', selectedBytes: '100' }], totalSelectedCount: '1', totalSelectedBytes: '100' });
      await Promise.all([previewCalls[0].promise, previewCalls[1].promise]);
    });

    await waitFor(() => expect(screen.getByText('Preview Breakdown')).toBeInTheDocument());
    const breakdown = screen.getByText('Preview Breakdown').closest('.preview-breakdown') as HTMLElement;
    function expectAnalyzerPreviewBreakdownOneItem() {
      expect(within(breakdown).getByText(/1 item/i)).toBeInTheDocument();
      const size = within(breakdown).getAllByText(/100 B/).length;
      expect(size).toBeGreaterThan(0);
      expect(within(breakdown).queryByText(/2 items/i)).not.toBeInTheDocument();
      const missing = within(breakdown).queryByText(/200 B/);
      expect(missing).not.toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled();
    }
    expectAnalyzerPreviewBreakdownOneItem();
  });

  it('AnalyzerView escapes ASCII and typographic smart single quotes in VHDX PowerShell command', async () => {
    const user = userEvent.setup();
    const maliciousPath = "C:\\VMs\\evil'\u2018\u2019\u201A\u201B$(calc).vhdx";
    const escapedPath = "C:\\VMs\\evil''\u2018\u2018\u2019\u2019\u201A\u201A\u201B\u201B$(calc).vhdx";
    const analyzerResult: StartAnalyzerResult = {
      sessionId: 'an-vhdx-smart', rootPath: 'C:\\', totalBytes: '10', totalItems: '1', scannedDirs: '1', scannedFiles: '1', cancelled: false, warnings: [],
      root: { id: 'root', name: 'C:', path: 'C:\\', isDir: true, sizeBytes: '10', itemCount: '1', risk: 'Safe', description: 'root', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: false, selectable: false, children: [
        { id: 'v1', name: 'evil.vhdx', path: maliciousPath, isDir: false, sizeBytes: '10', itemCount: '1', risk: 'Caution', description: 'vhdx', reason: null, tier: null, targetId: null, isLinkPoint: false, isCloudPlaceholder: false, displayOnly: true, selectable: false, children: [] },
      ] },
    };
    const client = {
      startAnalyzer: async () => analyzerResult,
      explainItem: async (itemId: string) => ({ itemId, title: 'evil.vhdx', description: 'desc', risk: 'Caution' as const }),
    } as unknown as SweeperClient;

    render(<AnalyzerView client={client} settings={{ allowDeleteUnknown: false } as any} onOpenLog={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    await screen.findByRole('heading', { name: 'Analyzer Tree' });
    const vhdxRow = (await screen.findAllByText(/evil\.vhdx/i))[0].closest('[role="treeitem"]') as HTMLElement;
    await user.click(vhdxRow);
    const expectedCmd = `wsl --shutdown\nOptimize-VHD -Path '${escapedPath}' -Mode Full`;
    await waitFor(() => expect(screen.getByText(/Optimize-VHD -Path/).closest('code')?.textContent).toBe(expectedCmd));
  });
});
