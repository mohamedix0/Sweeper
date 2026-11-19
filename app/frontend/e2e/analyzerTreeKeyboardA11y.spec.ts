import { expect, test, type Page } from '@playwright/test';

/**
 * Real-browser a11y coverage for issue #103 (mock sandbox only, no real C:
 * paths and no deletion). Playwright uses the existing system Edge channel
 * from playwright.config.ts; no browser is downloaded.
 *
 * Asserts the actual focus contract: DOM focus stays on role=tree while
 * aria-activedescendant tracks the active treeitem under arrow keys, and
 * focus is restored to the replacement Show more / Show fewer control.
 */
type TreeState = {
  activeElementRole: string | null;
  activeDescendantId: string | null;
  activeDescendantRole: string | null;
  treeTabIndex: string | null;
  treeitemIds: string[];
};

async function readTreeState(page: Page): Promise<TreeState> {
  return page.evaluate(() => {
    const tree = document.querySelector<HTMLElement>('[role="tree"][aria-label="Analyzer results tree"]');
    const activeDescendantId = tree?.getAttribute('aria-activedescendant') ?? null;
    const activeDescendant = activeDescendantId ? document.getElementById(activeDescendantId) : null;
    const treeitems = Array.from(document.querySelectorAll<HTMLElement>('[role="treeitem"]'));
    return {
      activeElementRole: document.activeElement?.getAttribute('role') ?? null,
      activeDescendantId,
      activeDescendantRole: activeDescendant?.getAttribute('role') ?? null,
      treeTabIndex: tree?.getAttribute('tabindex') ?? null,
      treeitemIds: treeitems.map((treeitem) => treeitem.id),
    };
  });
}

async function activeElementText(page: Page): Promise<string> {
  return page.evaluate(() => document.activeElement?.textContent?.trim() ?? '');
}

async function openAnalyzerTree(page: Page) {
  await page.goto('/');
  await page.getByRole('button', { name: 'Analyzer' }).click();
  await page.getByRole('button', { name: 'Analyze', exact: true }).click();
  const tree = page.getByRole('tree', { name: 'Analyzer results tree' });
  await expect(tree).toBeVisible({ timeout: 30_000 });
  await expect.poll(async () => (await readTreeState(page)).activeDescendantId).not.toBeNull();
  return tree;
}

test.describe('Analyzer tree keyboard accessibility (mock sandbox, system Edge)', () => {
  test('arrow keys move aria-activedescendant while DOM focus stays on role=tree', async ({ page }) => {
    const tree = await openAnalyzerTree(page);
    await tree.focus();

    let state = await readTreeState(page);
    expect(state.treeTabIndex).toBe('0');
    expect(state.activeElementRole).toBe('tree');
    expect(state.activeDescendantRole).toBe('treeitem');
    const startIndex = state.treeitemIds.indexOf(state.activeDescendantId ?? '');
    expect(startIndex).toBeGreaterThanOrEqual(0);

    await page.keyboard.press('ArrowDown');
    await expect
      .poll(async () => {
        const current = await readTreeState(page);
        return { role: current.activeElementRole, index: current.treeitemIds.indexOf(current.activeDescendantId ?? '') };
      })
      .toEqual({ role: 'tree', index: startIndex + 1 });

    await page.keyboard.press('ArrowDown');
    await expect
      .poll(async () => {
        const current = await readTreeState(page);
        return { role: current.activeElementRole, index: current.treeitemIds.indexOf(current.activeDescendantId ?? '') };
      })
      .toEqual({ role: 'tree', index: startIndex + 2 });

    await page.keyboard.press('ArrowUp');
    await expect
      .poll(async () => {
        const current = await readTreeState(page);
        return { role: current.activeElementRole, index: current.treeitemIds.indexOf(current.activeDescendantId ?? '') };
      })
      .toEqual({ role: 'tree', index: startIndex + 1 });

    state = await readTreeState(page);
    expect(state.activeElementRole).toBe('tree');
    expect(state.activeDescendantRole).toBe('treeitem');
  });

  test('focus returns to the replacement Show fewer / Show more control', async ({ page }) => {
    await openAnalyzerTree(page);

    // The mock backend's "bulk" folder has 40 children, so expanding it
    // inserts a Show more pseudo-row (the top 25 are shown by default).
    await page.getByRole('button', { name: 'Expand Temp' }).click();
    await page.getByRole('button', { name: 'Expand bulk' }).click();

    const showMore = page.getByRole('button', { name: /Show more/ });
    await expect(showMore).toBeVisible();
    await showMore.focus();
    await showMore.click();

    const showFewer = page.getByRole('button', { name: 'Show fewer files' });
    await expect(showFewer).toBeVisible();
    await expect(showMore).toHaveCount(0);
    await expect.poll(() => activeElementText(page)).toContain('Show fewer files');

    await showFewer.click();
    await expect(showMore).toBeVisible();
    await expect.poll(() => activeElementText(page)).toContain('Show more');
  });
});
