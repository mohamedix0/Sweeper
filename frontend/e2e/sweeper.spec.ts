import { expect, test } from '@playwright/test';

test.describe('Sweeper Phase 2 Scan & Preview (mock sandbox)', () => {
  test('full Scan -> Explain -> Preview breakdown with typed confirmation and error recovery', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Scan & Preview' })).toBeVisible();

    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Scan Results' })).toBeVisible();
    await expect(page.getByText('User Temp')).toBeVisible();

    // Explain
    await page.getByRole('button', { name: 'Explain' }).first().click();
    await expect(page.getByRole('dialog', { name: /Explain:/ })).toBeVisible();
    await expect(page.getByText(/Risk: Safe/)).toBeVisible();
    await page.keyboard.press('Escape');
    // fallback close if Escape not handled
    if (await page.getByRole('dialog').isVisible()) {
      await page.getByRole('button', { name: 'Close explanation' }).click();
    }
    await expect(page.getByRole('dialog')).toHaveCount(0);

    // Select safe item and unknown item (requires typed confirmation)
    await page.getByLabel('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp').check();
    await page.getByLabel('Select C:\\Users\\Alice\\AppData\\Local\\SomeApp\\MysteryCache\\old.bin').check();
    await expect(page.getByLabel('Typed confirmation')).toBeVisible();

    // typed confirmation error recovery
    await page.getByLabel('Typed confirmation').fill('NOPE');
    await page.getByRole('button', { name: 'Preview Clean (read-only)' }).click();
    await expect(page.getByRole('alert')).toContainText(/Typed confirmation required/i);

    await page.getByLabel('Typed confirmation').fill('DELETE');
    await page.getByRole('button', { name: 'Preview Clean (read-only)' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
    await expect(page.getByText(/mock-preview-/)).toBeVisible();
    await expect(page.getByRole('table', { name: 'Preview breakdown by Target' })).toBeVisible();
    await expect(page.locator('p', { hasText: 'Total selected:' })).toContainText('item(s)');
    await expect(page.getByRole('button', { name: 'Clean (Phase 3 — locked)' })).toBeDisabled();
  });

  test('Settings and Logs navigation', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings (Read-Only)' }).click();
    await expect(page.getByRole('heading', { name: 'Settings (Read-Only in Phase 2)' })).toBeVisible();
    await expect(page.getByText('Minimum Age (hours)')).toBeVisible();
    await expect(page.getByText('Allow deleting unknown folders')).toBeVisible();

    await page.getByRole('button', { name: 'Logs' }).click();
    await expect(page.getByRole('heading', { name: 'Logs', exact: true })).toBeVisible();
    await expect(page.getByRole('table', { name: 'Log list' })).toBeVisible();
    const firstRow = page.getByRole('row', { name: /2026-11-13-scan\.jsonl/ });
    await firstRow.getByRole('button', { name: 'View' }).click();
    await expect(page.getByLabel('Log content')).toBeVisible();
    await expect(page.getByLabel('Log content')).toContainText('"event":"scan"');
  });

  test('basic accessibility: labels, roles, and keyboard focus', async ({ page }) => {
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Primary' });
    await expect(nav).toBeVisible();
    await expect(page.getByLabel('Tier 1 (system temp and caches)')).toBeVisible();
    await expect(page.getByLabel('Tier 2 (developer caches)')).toBeVisible();

    // keyboard focus moves to Start Scan and activates
    await page.getByRole('button', { name: 'Start Scan' }).focus();
    await expect(page.getByRole('button', { name: 'Start Scan' })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('heading', { name: 'Scan Results' })).toBeVisible();

    // keyboard navigation reaches nav buttons
    await page.getByRole('button', { name: 'Logs' }).focus();
    await expect(page.getByRole('button', { name: 'Logs' })).toBeFocused();
  });
});
