import { expect, test, type Page } from '@playwright/test';

/**
 * DIAG-3a/3b-style mock-backend coverage for the post-Preview delete action.
 * No real deletion paths are used and the Edge channel comes from the shared
 * Playwright config; the browser mock backend is the only data source.
 */

type DeferredExplanations = {
  explainItem: (itemId: string) => Promise<unknown>;
  release: () => void;
};

async function runScan(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Start Scan' }).click();
  await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
}

async function expectedDeleteLabel(page: Page): Promise<string> {
  const summary = page.locator('.scan-bottom-bar-summary');
  const count = await summary.locator('strong.num').innerText();
  const size = await summary.locator('span.num').innerText();
  return `Delete ${count.replace(' selected', '')} permanently · ${size}`;
}

/** Hold every explanation until the test releases the gate, keeping Clean gated. */
async function installPendingExplanations(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const mock = (window as unknown as { __sweeperMock?: { explainItem?: unknown } }).__sweeperMock;
    return typeof mock?.explainItem === 'function';
  });

  await page.evaluate(() => {
    const mock = (window as unknown as { __sweeperMock: DeferredExplanations }).__sweeperMock;
    const original = mock.explainItem.bind(mock);
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    mock.explainItem = async (itemId: string) => {
      await gate;
      return original(itemId);
    };
    (window as unknown as { __releaseSweeperExplanations: () => void }).__releaseSweeperExplanations = release;
  });
}

test.describe('Sweeper post-Preview delete action (mock sandbox)', () => {
  test('DIAG-3a: after Preview the CTA is prominent, exact, enabled and performs the single clean', async ({ page }) => {
    await page.goto('/');
    await runScan(page);

    const expected = await expectedDeleteLabel(page);
    await page.getByRole('button', { name: 'Review & Clean' }).click();

    const cta = page.getByRole('button', { name: 'Clean Permanently' });
    await expect(cta).toBeVisible();
    await expect(cta).toBeEnabled();
    await expect(cta).toContainText(expected);
    await expect(page.getByText(/Preparing safety review/)).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Unselect all' })).toHaveClass(/subtle-btn/);

    await cta.click();
    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
  });

  test('DIAG-3b: while the safety review is pending an explicit reason is shown, then the CTA enables', async ({ page }) => {
    await page.goto('/');
    await installPendingExplanations(page);
    await runScan(page);

    await page.getByRole('button', { name: 'Review & Clean' }).click();

    const pending = page.getByText(/Preparing safety review…/);
    await expect(pending).toBeVisible();
    await expect(pending).toContainText('items');
    const cta = page.getByRole('button', { name: 'Clean Permanently' });
    await expect(cta).toBeDisabled();

    await page.evaluate(() => {
      (window as unknown as { __releaseSweeperExplanations: () => void }).__releaseSweeperExplanations();
    });

    await expect(cta).toBeEnabled();
    await expect(page.getByText(/Preparing safety review…/)).toHaveCount(0);
  });
});
