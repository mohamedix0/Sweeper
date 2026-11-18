import { expect, test, type Page } from '@playwright/test';
import { installPreviewCalloutMock } from './previewCalloutMock';

/**
 * Mock-backend E2E coverage for the Preview per-item Risk callout (#44/#42).
 * Paths use the repository's canonical fictional user (Alice); no real Clean
 * is performed and the standard Edge channel comes from playwright.config.ts.
 */
const CALLOUT_GROUP = 'Elevated risk in this selection';
const CAUTION_IDENTITY = 'Vendor Logs';
const CAUTION_DESCRIPTION = 'Knowledge Base: vendor application log, regenerated on demand.';
const CAUTION_CONSEQUENCE = 'Removing this item is permanent and it may be slow or costly to restore.';
const NPM_ITEM_ID = 't-npm-item-1';
const NPM_ITEM_LABEL = 'Select C:\\Users\\Alice\\AppData\\Local\\npm-cache\\package-cache-tarballs';
const SKIPPED_REASON = 'Recent file (locked) — Skipped';

async function runScan(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Start Scan' }).click();
  await expect(page.getByRole('heading', { name: 'Inspector' })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByText('Ready to scan')).toHaveCount(0);
}

/** Unselect the auto-selected Safe items, then select only the npm-cache item. */
async function selectOnlyNpmItem(page: Page): Promise<void> {
  const unselectAll = page.getByRole('button', { name: 'Unselect all' });
  if (await unselectAll.isVisible().catch(() => false)) await unselectAll.click();

  const group = page.getByRole('article').filter({ hasText: 'npm Package Cache' }).first();
  const expand = group.getByRole('button', { name: 'Expand npm Package Cache' });
  if (await expand.isVisible().catch(() => false)) await expand.click();
  await group.getByLabel(NPM_ITEM_LABEL).check();
}

test.describe('Sweeper Preview Risk callouts (mock sandbox)', () => {
  test('Caution item shows a callout with accessible role and identity, beside the breakdown and skipped notice', async ({ page }) => {
    await page.goto('/');
    await installPreviewCalloutMock(page, {
      elevatedItemId: NPM_ITEM_ID,
      elevatedTitle: CAUTION_IDENTITY,
      elevatedDescription: CAUTION_DESCRIPTION,
      skippedCount: '1',
      skippedReasons: [SKIPPED_REASON],
    });

    await runScan(page);
    await selectOnlyNpmItem(page);
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
    await expect(page.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled();
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
    await expect(page.getByRole('button', { name: 'Clean Permanently' })).toHaveCount(0);
    await expect(page.getByRole('group', { name: CALLOUT_GROUP })).toHaveCount(0);

    await typed.fill('delete');
    await expect(review).toBeDisabled();
    await typed.fill('DELETE');
    await expect(review).toBeEnabled();
    await review.click();

    // The selection that produced a callout still required the typed phrase.
    const callouts = page.getByRole('group', { name: CALLOUT_GROUP });
    await expect(callouts).toBeVisible();
    await expect(callouts.getByRole('note', { name: 'Custom Path risk: Custom Path (Unknown)' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled();
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
    await expect(page.getByRole('button', { name: 'Clean Permanently' })).toBeEnabled();
  });

  test('cancelling a Preview that has callouts returns to the normal state', async ({ page }) => {
    await page.goto('/');
    await installPreviewCalloutMock(page, {
      elevatedItemId: NPM_ITEM_ID,
      elevatedTitle: CAUTION_IDENTITY,
      elevatedDescription: CAUTION_DESCRIPTION,
    });

    await runScan(page);
    await selectOnlyNpmItem(page);
    await page.getByRole('button', { name: 'Review & Clean' }).click();

    const group = page.getByRole('group', { name: CALLOUT_GROUP });
    await expect(group).toBeVisible();
    await expect(group.getByRole('note', { name: `Caution risk: ${CAUTION_IDENTITY}` })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toBeVisible();

    await page.getByRole('button', { name: 'Unselect all' }).click();

    await expect(group).toHaveCount(0);
    await expect(page.getByRole('note')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Preview Breakdown' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Clean Permanently' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Review & Clean' })).toBeVisible();
  });
});
