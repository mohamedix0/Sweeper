import { expect, test, type Page } from '@playwright/test';

/**
 * Mock-fidelity coverage for issue #100: Skipped preview items and an aborted
 * Clean. All data comes from the in-memory browser mock backend via
 * `window.__sweeperMock`; no real filesystem or `C:` path is touched, and the
 * Edge channel comes from the shared playwright.config.ts.
 */

type InvalidationOptions = { reason?: string; abort?: boolean };
type MockWithInvalidation = { invalidateItem: (itemId: string, options?: InvalidationOptions) => void };

const USER_TEMP_ITEM = 't-user-temp-item-1';
const CRASH_ITEM = 't-crash-item-1';

async function waitForInvalidationHook(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    const mock = (window as unknown as { __sweeperMock?: { invalidateItem?: unknown } }).__sweeperMock;
    return typeof mock?.invalidateItem === 'function';
  });
}

async function invalidateItem(page: Page, itemId: string, options?: InvalidationOptions): Promise<void> {
  await page.evaluate(
    ({ id, opts }) => {
      const mock = (window as unknown as { __sweeperMock: MockWithInvalidation }).__sweeperMock;
      mock.invalidateItem(id, opts);
    },
    { id: itemId, opts: options },
  );
}

async function runScan(page: Page): Promise<void> {
  await page.goto('/');
  await waitForInvalidationHook(page);
  await page.getByRole('button', { name: 'Start Scan' }).click();
  await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText('Ready to scan')).toHaveCount(0);
}

test.describe('Sweeper Skipped preview and aborted Clean (mock sandbox)', () => {
  test('Preview reports Skipped and drops the invalid item; Clean emits a typed Skipped reason', async ({ page }) => {
    await runScan(page);

    // Three Safe items are auto-selected. Invalidate one before Preview so it
    // is dropped from the selectable set and surfaced as Skipped metadata.
    await invalidateItem(page, USER_TEMP_ITEM, { reason: 'File locked' });
    await page.getByRole('button', { name: 'Review & Clean' }).click();

    const skipped = page.getByRole('status', { name: 'Preview skipped items' });
    await expect(skipped).toBeVisible();
    await expect(skipped).toContainText('Skipped during preview');
    await expect(skipped).toContainText('File locked');

    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();
    const rows = page.getByRole('table', { name: 'Preview breakdown by Target' }).locator('tbody tr');
    await expect(rows).toHaveCount(2);
    await expect(page.getByText(/Total selected: 2 items/)).toBeVisible();

    // Invalidate a token item after Preview so Clean re-validation skips it
    // with a typed reason instead of deleting it.
    await invalidateItem(page, CRASH_ITEM, { reason: 'Failed re-validation' });
    await page.getByRole('button', { name: /Delete .* permanently/ }).click();

    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
    const counts = page.locator('.kv dd.num');
    await expect(counts.nth(1)).toHaveText('1'); // Deleted count (one item was dropped at Preview, one skipped at Clean)
    await expect(counts.nth(2)).toHaveText('1'); // Skipped count
    await expect(page.getByLabel('Skipped reasons')).toContainText('Skipped: Failed re-validation');
  });

  test('Clean honours an invalidation after Preview and renders the aborted branch', async ({ page }) => {
    await runScan(page);

    await page.getByRole('button', { name: 'Review & Clean' }).click();
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();
    await expect(page.getByRole('status', { name: 'Preview skipped items' })).toHaveCount(0);

    // Invalidate between Preview and Clean with abort semantics: the token
    // stays valid but moment-of-deletion re-validation aborts the whole Clean.
    await invalidateItem(page, USER_TEMP_ITEM, { reason: 'Circuit breaker exceeded', abort: true });
    await page.getByRole('button', { name: /Delete .* permanently/ }).click();

    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
    const aborted = page.getByRole('alert').filter({ hasText: 'Clean aborted.' });
    await expect(aborted).toBeVisible();
    await expect(aborted).toContainText('aborted: Circuit breaker exceeded');

    const counts = page.locator('.kv dd.num');
    await expect(counts.nth(1)).toHaveText('0'); // Deleted count
    await expect(page.getByLabel('Skipped reasons')).toContainText('Skipped: Circuit breaker exceeded');
  });
});
