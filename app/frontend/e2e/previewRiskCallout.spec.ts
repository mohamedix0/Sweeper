import { expect, test, type Page } from '@playwright/test';
import { installPreviewSkippedMock } from './previewCalloutMock';

/**
 * Mock-backend E2E coverage for the Preview per-item Risk callout (#44/#42).
 *
 * The elevated item is a genuine Caution Risk Target from the mock Knowledge
 * Base (`CAUTION_VENDOR_LOGS`), so its identity, Risk and explanation all come
 * from the real mock backend with no `explainItem` override. Paths use the
 * repository's canonical fictional user (Alice / ProgramData); no real Clean is
 * performed and the standard Edge channel comes from playwright.config.ts.
 */
const CALLOUT_GROUP = 'Elevated risk in this selection';
const CAUTION_GROUP_LABEL = 'Vendor Logs';
const CAUTION_IDENTITY = 'C:\\ProgramData\\Vendor\\Logs\\app.log';
const CAUTION_ITEM_LABEL = `Select ${CAUTION_IDENTITY}`;
const CAUTION_DESCRIPTION = 'Vendor application logs under ProgramData. Regenerated on demand; removal is safe but historical diagnostics are lost.';
const CAUTION_CONSEQUENCE = 'Removing this item is permanent and it may be slow or costly to restore.';
const CUSTOM_PATH_IDENTITY = 'C:\\Users\\Alice\\CustomCleanup\\old.bin';
const SKIPPED_REASON = 'Recent file (locked) — Skipped';

async function runScan(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Start Scan' }).click();
  await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText('Ready to scan')).toHaveCount(0);
}

/** Unselect the auto-selected Safe items, then select only the genuine Caution item. */
async function selectOnlyCautionItem(page: Page): Promise<void> {
  const unselectAll = page.getByRole('button', { name: 'Unselect all' });
  if (await unselectAll.isVisible().catch(() => false)) await unselectAll.click();

  const group = page.getByRole('article').filter({ hasText: CAUTION_GROUP_LABEL }).first();
  const expand = group.getByRole('button', { name: `Expand ${CAUTION_GROUP_LABEL}` });
  if (await expand.isVisible().catch(() => false)) await expand.click();
  await group.getByLabel(CAUTION_ITEM_LABEL).check();
}

test.describe('Sweeper Preview Risk callouts (mock sandbox)', () => {
  test('genuine Caution item shows a callout with its Knowledge Base explanation, beside the breakdown and skipped notice', async ({ page }) => {
    await page.goto('/');
    await installPreviewSkippedMock(page, {
      skippedCount: '1',
      skippedReasons: [SKIPPED_REASON],
    });

    await runScan(page);
    await selectOnlyCautionItem(page);
    await expect(page.getByRole('button', { name: 'Review & Clean' })).toBeEnabled();
    await page.getByRole('button', { name: 'Review & Clean' }).click();

    const group = page.getByRole('group', { name: CALLOUT_GROUP });
    await expect(group).toBeVisible();
    const note = group.getByRole('note', { name: `Caution risk: ${CAUTION_IDENTITY}` });
    await expect(note).toBeVisible();
    await expect(note.getByText('Caution', { exact: true })).toBeVisible();
    await expect(note.getByText('Warning', { exact: true })).toBeVisible();
    await expect(note.getByText(CAUTION_IDENTITY, { exact: true })).toBeVisible();
    await expect(note.getByText(CAUTION_DESCRIPTION)).toBeVisible();
    await expect(note.getByText(CAUTION_CONSEQUENCE)).toBeVisible();

    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();
    await expect(page.getByRole('table', { name: 'Preview breakdown by Target' })).toBeVisible();
    const skipped = page.getByRole('status', { name: 'Preview skipped items' });
    await expect(skipped).toBeVisible();
    await expect(skipped).toContainText('Skipped during preview');
    await expect(skipped).toContainText(SKIPPED_REASON);

    // The callout is presentational: the normal Clean control stays available.
    await expect(page.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled();
  });

  test('typed confirmation remains mandatory and the callout never enables Clean on its own', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    await page.getByLabel('Allow deleting unknown folders (advanced)').check();
    await page.getByRole('button', { name: 'Save Settings' }).first().click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();
    await page.getByRole('button', { name: /Scan & (Clean|Preview)/ }).click();

    await runScan(page);
    const customGroup = page.getByRole('article').filter({ hasText: 'Custom Path (Unknown)' }).first();
    await expect(customGroup).toBeVisible();
    const expand = customGroup.getByRole('button', { name: 'Expand Custom Path (Unknown)' });
    if (await expand.isVisible().catch(() => false)) await expand.click();
    await customGroup.getByLabel('Select C:\\Users\\Alice\\CustomCleanup\\old.bin').check();

    const review = page.getByRole('button', { name: 'Review & Clean' });
    const typed = page.getByLabel('Typed confirmation');
    await expect(typed).toBeVisible();
    await expect(review).toBeDisabled();
    await expect(page.getByRole('button', { name: /Delete .* permanently/ })).toHaveCount(0);
    await expect(page.getByRole('group', { name: CALLOUT_GROUP })).toHaveCount(0);

    await typed.fill('delete');
    await expect(review).toBeDisabled();
    await typed.fill('DELETE');
    await expect(review).toBeEnabled();
    await review.click();

    // The selection that produced a callout still required the typed phrase.
    const callouts = page.getByRole('group', { name: CALLOUT_GROUP });
    await expect(callouts).toBeVisible();
    await expect(callouts.getByRole('note', { name: `Custom Path risk: ${CUSTOM_PATH_IDENTITY}` })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();
    await expect(page.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled();
  });

  test('Safe-only selection renders no callout and the existing Preview flow is unchanged', async ({ page }) => {
    await page.goto('/');
    await runScan(page);

    await expect(page.getByRole('button', { name: 'Review & Clean' })).toBeEnabled();
    await page.getByRole('button', { name: 'Review & Clean' }).click();

    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();
    await expect(page.getByRole('table', { name: 'Preview breakdown by Target' })).toBeVisible();
    await expect(page.getByRole('group', { name: CALLOUT_GROUP })).toHaveCount(0);
    await expect(page.getByRole('note')).toHaveCount(0);
    await expect(page.getByLabel('Typed confirmation')).toHaveCount(0);
    await expect(page.getByRole('button', { name: /Delete .* permanently/ })).toBeEnabled();
  });

  test('cancelling a Preview that has callouts returns to the normal state', async ({ page }) => {
    await page.goto('/');

    await runScan(page);
    await selectOnlyCautionItem(page);
    await page.getByRole('button', { name: 'Review & Clean' }).click();

    const group = page.getByRole('group', { name: CALLOUT_GROUP });
    await expect(group).toBeVisible();
    await expect(group.getByRole('note', { name: `Caution risk: ${CAUTION_IDENTITY}` })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();

    await page.getByRole('button', { name: 'Unselect all' }).click();

    await expect(group).toHaveCount(0);
    await expect(page.getByRole('note')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: /Delete .* permanently/ })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Review & Clean' })).toBeVisible();
  });
});
