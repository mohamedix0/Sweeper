import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
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

function threeFileResult(): StartAnalyzerResult {
  return makeAnalyzerResult({
    sessionId: 'a11y-three',
    root: makeAnalyzerNode({
      id: 'root',
      name: 'C:\\',
      path: 'C:\\',
      isDir: true,
      children: [
        makeAnalyzerNode({ id: 'a', name: 'a.tmp', path: 'C:\\Temp\\a.tmp', sizeBytes: '300' }),
        makeAnalyzerNode({ id: 'b', name: 'b.tmp', path: 'C:\\Temp\\b.tmp', sizeBytes: '200' }),
        makeAnalyzerNode({ id: 'c', name: 'c.tmp', path: 'C:\\Temp\\c.tmp', sizeBytes: '100' }),
      ],
    }),
  });
}

function manyChildrenResult(count: number): StartAnalyzerResult {
  const children = Array.from({ length: count }, (_, i) =>
    makeAnalyzerNode({ id: `f-${i}`, name: `f-${i}.tmp`, path: `C:\\Temp\\f-${i}.tmp`, sizeBytes: String(count - i) }),
  );
  return makeAnalyzerResult({
    sessionId: 'a11y-many',
    root: makeAnalyzerNode({ id: 'root', name: 'C:\\', path: 'C:\\', isDir: true, children }),
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
});
