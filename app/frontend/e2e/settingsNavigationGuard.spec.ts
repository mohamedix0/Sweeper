import { expect, test, type Page } from '@playwright/test';

async function openSettings(page: Page): Promise<void> {
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
}

test('navigating with no unsaved Settings changes does not prompt', async ({ page }) => {
  await openSettings(page);
  await expect(page.getByText('All changes saved')).toBeVisible();

  await page.getByRole('button', { name: 'Scan & Clean' }).click();

  await expect(page.getByRole('heading', { name: 'Scan & Clean' })).toBeVisible();
  await expect(page.getByRole('alertdialog')).toHaveCount(0);
});

test('a dirty Settings draft prompts, is kept when staying, and is discarded only on explicit confirmation', async ({ page }) => {
  await openSettings(page);
  await page.getByLabel('Enable Tier 1').uncheck();
  await expect(page.getByLabel('Enable Tier 1')).not.toBeChecked();

  await page.getByRole('button', { name: 'Activity' }).click();
  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText(/unsaved/i);
  // Navigation is held until the user chooses.
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();

  await dialog.getByRole('button', { name: 'Keep editing' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByLabel('Enable Tier 1')).not.toBeChecked();

  await page.getByRole('button', { name: 'Activity' }).click();
  await dialog.getByRole('button', { name: 'Discard changes' }).click();
  await expect(page.getByRole('heading', { name: 'Activity History' })).toBeVisible();

  // The unsaved draft is gone: returning to Settings shows the saved value.
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByLabel('Enable Tier 1')).toBeChecked();
});
