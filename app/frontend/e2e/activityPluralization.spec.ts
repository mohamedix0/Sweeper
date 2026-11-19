import { expect, test, type Page } from '@playwright/test';

/**
 * E2E coverage for correct singular/plural text in the Activity view (#70.3).
 * Log fixtures are installed on the browser mock backend only; no real log or
 * filesystem path is read, and Edge comes from the shared Playwright config.
 */
type LogFixture = { id: string; content: string };

async function installLogs(page: Page, fixtures: LogFixture[]): Promise<void> {
  await page.waitForFunction(() => {
    const mock = (window as unknown as { __sweeperMock?: { listLogs?: unknown; readLog?: unknown } }).__sweeperMock;
    return typeof mock?.listLogs === 'function' && typeof mock?.readLog === 'function';
  });

  await page.evaluate((entries) => {
    const mock = (window as unknown as {
      __sweeperMock: {
        listLogs: () => Promise<Array<{ id: string; name: string; sizeBytes: string; modifiedUnixSecs: string }>>;
        readLog: (id: string) => Promise<{ id: string; content: string }>;
      };
    }).__sweeperMock;
    mock.listLogs = async () => entries.map((entry) => ({
      id: entry.id,
      name: `${entry.id}.jsonl`,
      sizeBytes: String(entry.content.length),
      modifiedUnixSecs: '1710000000',
    }));
    mock.readLog = async (id) => {
      const found = entries.find((entry) => entry.id === id);
      if (!found) throw new Error('Log not found.');
      return { id, content: found.content };
    };
  }, fixtures);
}

async function openActivity(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Activity' }).click();
  await expect(page.getByRole('heading', { name: 'Activity History' })).toBeVisible();
}

const scanLine = (counts: { categories: number; files: number; skipped?: number }) =>
  JSON.stringify({
    record: 'outcome', event: 'scan', status: 'completed',
    categories_count: counts.categories,
    total_eligible_files: counts.files,
    total_skipped_files: counts.skipped ?? 0,
  });

test.describe('Sweeper Activity pluralization (mock sandbox)', () => {
  test('a single activity and single file/category read as singular', async ({ page }) => {
    await page.goto('/');
    await installLogs(page, [{ id: 'scan-one', content: scanLine({ categories: 1, files: 1 }) }]);
    await openActivity(page);

    await expect(page.getByText('1 activity', { exact: true })).toBeVisible();
    const summary = page.getByLabel('Activity summary');
    await expect(summary).toContainText('1 file · 1 category');
    await expect(summary).not.toContainText('1 categories');
    await expect(page.getByText('1 category', { exact: true })).toBeVisible();
    await expect(page.getByText('1 categories', { exact: false })).toHaveCount(0);
  });

  test('multiple activities and files/categories read as plural', async ({ page }) => {
    await page.goto('/');
    await installLogs(page, [
      { id: 'scan-many', content: scanLine({ categories: 2, files: 3, skipped: 2 }) },
      { id: 'scan-other', content: scanLine({ categories: 1, files: 1 }) },
    ]);
    await openActivity(page);

    await expect(page.getByText('2 activities', { exact: true })).toBeVisible();
    const summary = page.getByLabel('Activity summary');
    await expect(summary).toContainText('3 files · 2 categories');
    await expect(summary).toContainText('2 files · 0 categories');
    await expect(page.getByText('2 categories', { exact: true })).toBeVisible();
  });
});
