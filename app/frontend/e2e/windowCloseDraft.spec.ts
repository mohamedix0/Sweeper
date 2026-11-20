import { expect, test, type Page } from '@playwright/test';

// A native window close cannot be produced from a browser page, so this E2E
// drives the webview half of the handshake with the browser-preview CustomEvent
// that mirrors the Rust host event. The Rust CloseRequested path itself is
// covered independently by the `gate_f_close` backend tests.

async function openSettings(page: Page): Promise<void> {
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
}

async function requestClose(page: Page): Promise<void> {
  await page.evaluate(() => window.dispatchEvent(new CustomEvent('sweeper:close-requested')));
}

test('a clean close is approved without any prompt', async ({ page }) => {
  await openSettings(page);
  await requestClose(page);

  await expect(page.getByRole('alertdialog')).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as { sweeperCloseDecision?: boolean }).sweeperCloseDecision))
    .toBe(true);
});

test('a dirty close can be cancelled without losing the edit', async ({ page }) => {
  await openSettings(page);
  const tier1 = page.getByLabel('Enable Tier 1');
  await tier1.uncheck();
  await requestClose(page);

  const dialog = page.getByRole('alertdialog');
  await expect(dialog).toContainText(/unsaved/i);
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toHaveCount(0);

  await expect
    .poll(() => page.evaluate(() => (window as { sweeperCloseDecision?: boolean }).sweeperCloseDecision))
    .toBe(false);
  await expect(tier1).not.toBeChecked();
});

test('a dirty close saves the draft before approving, with no silent loss', async ({ page }) => {
  await openSettings(page);
  const tier1 = page.getByLabel('Enable Tier 1');
  await tier1.uncheck();
  await requestClose(page);

  const dialog = page.getByRole('alertdialog');
  await dialog.getByRole('button', { name: 'Save & close' }).click();
  await expect(dialog).toHaveCount(0);

  await expect
    .poll(() => page.evaluate(() => (window as { sweeperCloseDecision?: boolean }).sweeperCloseDecision))
    .toBe(true);

  // Re-opening Settings shows the persisted value, proving the edit survived.
  await page.getByRole('button', { name: 'Scan & Clean' }).click();
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByLabel('Enable Tier 1')).not.toBeChecked();
});
