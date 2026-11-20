import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AnalyzerView } from './components/AnalyzerView';
import type { StartAnalyzerResult, SweeperClient } from './api/client';
import { makeAnalyzerNode, makeAnalyzerResult, makeSettings } from './test/fixtures';

function clientFor(result: StartAnalyzerResult): SweeperClient {
  return {
    startScan: async () => ({ sessionId: 'scan-a11y' }),
    getScanResults: async () => ({ sessionId: 'scan-a11y', startedUnixSecs: '0', minAgeHours: 24, targets: [] }),
    previewClean: async () => ({ previewToken: 'tok', breakdown: [], totalSelectedCount: '0', totalSelectedBytes: '0' }),
    executeClean: async () => ({ bytesFreed: '0', deletedCount: '0', skippedCount: '0', skippedReasons: [], aborted: false, abortReason: null, logId: null }),
    startAnalyzer: async () => result,
    cancelAnalyzer: async () => undefined,
    explainItem: async (itemId) => ({ itemId, title: itemId, description: '', risk: 'Safe' }),
    getSettings: async () => makeSettings(),
    updateSettings: async (settings) => settings,
    getElevationStatus: async () => ({ isElevated: false }),
    relaunchElevated: async () => ({ launched: false }),
    validateCustomPath: async (path) => ({ path, allowed: true, reason: '' }),
    listLogs: async () => [],
    readLog: async (logId) => ({ id: logId, content: '' }),
  };
}

type TempFileFixture = { id: string; sizeBytes: string; selectable?: boolean };

/** One factory for every "C:\ root with Temp file children" fixture. */
function tempFileTreeResult(sessionId: string, files: readonly TempFileFixture[]): StartAnalyzerResult {
  return makeAnalyzerResult({
    sessionId,
    root: makeAnalyzerNode({
      id: 'root',
      name: 'C:\\',
      path: 'C:\\',
      isDir: true,
      children: files.map((file) => makeAnalyzerNode({
        id: file.id,
        name: `${file.id}.tmp`,
        path: `C:\\Temp\\${file.id}.tmp`,
        sizeBytes: file.sizeBytes,
        selectable: file.selectable,
        targetId: file.selectable ? 't1' : null,
      })),
    }),
  });
}

function threeFileResult(): StartAnalyzerResult {
  return tempFileTreeResult('a11y-three', [
    { id: 'a', sizeBytes: '300' },
    { id: 'b', sizeBytes: '200' },
    { id: 'c', sizeBytes: '100' },
  ]);
}

function manyChildrenResult(count: number): StartAnalyzerResult {
  return tempFileTreeResult('a11y-many', Array.from({ length: count }, (_, i) => ({
    id: `f-${i}`,
    sizeBytes: String(count - i),
  })));
}

function selectableFileResult(): StartAnalyzerResult {
  return tempFileTreeResult('a11y-activate', [
    { id: 'a', sizeBytes: '300', selectable: true },
    { id: 'b', sizeBytes: '200', selectable: true },
  ]);
}

function interactionResult(): StartAnalyzerResult {
  const child = makeAnalyzerNode({ id: 'kid', name: 'kid.tmp', path: 'C:\\Temp\\kid.tmp', sizeBytes: '50', selectable: true, targetId: 't1' });
  const temp = makeAnalyzerNode({
    id: 'temp', name: 'Temp', path: 'C:\\Temp', isDir: true, sizeBytes: '300',
    selectable: true, targetId: 't1', children: [child],
  });
  return makeAnalyzerResult({
    sessionId: 'a11y-interaction',
    root: makeAnalyzerNode({ id: 'root', name: 'C:\\', path: 'C:\\', isDir: true, children: [temp] }),
  });
}

async function openTree(result: StartAnalyzerResult) {
  const user = userEvent.setup();
  render(<AnalyzerView client={clientFor(result)} settings={makeSettings()} onOpenLog={() => undefined} />);
  await user.click(screen.getByRole('button', { name: 'Analyze' }));
  const tree = await screen.findByRole('tree', { name: 'Analyzer results tree' });
  return { user, tree, rows: within(tree).getAllByRole('treeitem') as HTMLElement[] };
}

describe('Analyzer tree accessibility', () => {
  it('keeps DOM focus on role=tree and tracks the active treeitem via aria-activedescendant on arrow keys', async () => {
    const { user, tree, rows } = await openTree(threeFileResult());
    expect(rows).toHaveLength(3);

    // Single, coherent focus model: the role=tree container owns DOM focus and
    // is the only tab stop; treeitems are never separately focused.
    expect(tree.getAttribute('tabindex')).toBe('0');
    expect(rows.every((row) => !row.hasAttribute('tabindex'))).toBe(true);

    tree.focus();
    expect(document.activeElement).toBe(tree);

    // aria-activedescendant resolves to a real treeitem, not just any node.
    const activeItem = (): HTMLElement | null => {
      const id = tree.getAttribute('aria-activedescendant');
      const element = document.getElementById(id ?? '');
      expect(element?.getAttribute('role')).toBe('treeitem');
      return element;
    };
    expect(activeItem()).toBe(rows[0]);

    await user.keyboard('{ArrowDown}');
    expect(document.activeElement).toBe(tree);
    expect(activeItem()).toBe(rows[1]);

    await user.keyboard('{ArrowDown}');
    expect(document.activeElement).toBe(tree);
    expect(activeItem()).toBe(rows[2]);

    // Navigation clamps at the ends and never moves DOM focus off the tree.
    await user.keyboard('{ArrowDown}');
    expect(document.activeElement).toBe(tree);
    expect(activeItem()).toBe(rows[2]);

    await user.keyboard('{ArrowUp}');
    expect(document.activeElement).toBe(tree);
    expect(activeItem()).toBe(rows[1]);

    await user.keyboard('{ArrowUp}');
    expect(activeItem()).toBe(rows[0]);

    await user.keyboard('{ArrowUp}');
    expect(document.activeElement).toBe(tree);
    expect(activeItem()).toBe(rows[0]);

    // Focus and selection semantics stay consistent: the active treeitem is
    // identified only by aria-activedescendant, never by a tabIndex on a row.
    expect(rows.filter((row) => row.getAttribute('tabindex') === '0')).toHaveLength(0);
  });

  it('restores focus to the replacement Show fewer / Show more control after toggling', async () => {
    const { user } = await openTree(manyChildrenResult(30));

    const showMore = screen.getByRole('button', { name: /Show more/ });
    showMore.focus();
    expect(document.activeElement).toBe(showMore);
    await user.click(showMore);

    const showFewer = screen.getByRole('button', { name: 'Show fewer files' });
    expect(screen.queryByRole('button', { name: /Show more/ })).not.toBeInTheDocument();
    expect(document.activeElement).toBe(showFewer);

    await user.click(showFewer);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: /Show more/ }));
  });

  it('scrolls the newly active treeitem into view with block:nearest on arrow navigation', async () => {
    const { user, tree, rows } = await openTree(threeFileResult());
    const scrolled: Array<{ id: string; arg: unknown }> = [];
    for (const row of rows) {
      row.scrollIntoView = ((arg: unknown) => { scrolled.push({ id: row.id, arg }); }) as typeof row.scrollIntoView;
    }

    tree.focus();
    await user.keyboard('{ArrowDown}');
    expect(scrolled).toEqual([{ id: rows[1].id, arg: { block: 'nearest' } }]);

    await user.keyboard('{ArrowUp}');
    expect(scrolled[scrolled.length - 1]).toEqual({ id: rows[0].id, arg: { block: 'nearest' } });
  });

  it('activates the active row with Enter and Space while DOM focus stays on the tree', async () => {
    const result = selectableFileResult();
    const explainItem = vi.fn(async (itemId: string) => ({ itemId, title: itemId, description: '', risk: 'Safe' as const }));
    const user = userEvent.setup();
    render(<AnalyzerView client={{ ...clientFor(result), explainItem }} settings={makeSettings()} onOpenLog={() => undefined} />);
    await user.click(screen.getByRole('button', { name: 'Analyze' }));
    const tree = await screen.findByRole('tree', { name: 'Analyzer results tree' });
    tree.focus();

    const firstCheckbox = screen.getByLabelText('Select analyzer node C:\\Temp\\a.tmp') as HTMLInputElement;
    expect(firstCheckbox).not.toBeChecked();

    await user.keyboard('{Enter}');
    expect(firstCheckbox).toBeChecked();
    expect(explainItem).toHaveBeenCalledWith('a');
    expect(document.activeElement).toBe(tree);

    await user.keyboard(' ');
    expect(firstCheckbox).not.toBeChecked();
    expect(document.activeElement).toBe(tree);
  });

  it('keeps row controls out of the tab order and exposes them via a tree interaction mode', async () => {
    const { user, tree } = await openTree(interactionResult());
    tree.focus();

    const controls = Array.from(tree.querySelectorAll<HTMLElement>('[data-tree-control]'));
    expect(controls.length).toBeGreaterThan(0);
    expect(controls.every((control) => control.getAttribute('tabindex') === '-1')).toBe(true);

    await user.keyboard('{ArrowRight}');
    expect(document.activeElement).toBe(controls[0]);

    await user.keyboard('{ArrowRight}');
    expect(document.activeElement).toBe(controls[1]);

    // Enter and Space activate the focused row control (the select checkbox).
    const select = controls[1] as HTMLInputElement;
    expect(select).not.toBeChecked();
    await user.keyboard('{Enter}');
    expect(select).toBeChecked();
    await user.keyboard(' ');
    expect(select).not.toBeChecked();

    await user.keyboard('{Escape}');
    expect(document.activeElement).toBe(tree);
  });
});
