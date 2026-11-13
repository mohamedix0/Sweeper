import { expect, test } from '@playwright/test';

test.describe('Sweeper Scan/Preview/Clean (mock sandbox)', () => {
  test('full Scan -> Explain -> Preview -> Clean summary -> Logs viewer flow', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByRole('heading', { name: 'Scan & Preview' })).toBeVisible();

    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Scan Results' })).toBeVisible();

    await page.getByRole('button', { name: 'Explain' }).first().click();
    await expect(page.getByRole('dialog', { name: /Explain:/ })).toBeVisible();
    await expect(page.getByText(/Risk: Safe/)).toBeVisible();
    await page.keyboard.press('Escape');
    if (await page.getByRole('dialog').isVisible()) {
      await page.getByRole('button', { name: 'Close explanation' }).click();
    }
    await expect(page.getByRole('dialog')).toHaveCount(0);

    await page.getByLabel('Select C:\\Users\\Alice\\AppData\\Local\\Temp\\a.tmp').check();
    await page.getByRole('button', { name: 'Preview Clean' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
    await expect(page.getByText(/Permanent deletion \(bypasses Recycle Bin\)/)).toBeVisible();

    await page.getByRole('button', { name: 'Clean Permanently' }).click();
    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
    await expect(page.getByText('Bytes freed')).toBeVisible();
    await expect(page.getByText('Deleted count')).toBeVisible();
    await expect(page.getByText('Skipped count')).toBeVisible();
    await expect(page.getByLabel('Skipped reasons')).toBeVisible();

    await page.getByRole('button', { name: 'View Operation Log' }).click();
    await expect(page.getByRole('heading', { name: 'Logs', exact: true })).toBeVisible();
    await expect(page.getByLabel('Log content')).toBeVisible();
    await expect(page.getByLabel('Log content')).toContainText('"record":"intent"');
  });

  test('typed confirmation and preview error recovery', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Scan Results' })).toBeVisible();

    await page.getByLabel('Select C:\\Users\\Alice\\AppData\\Local\\SomeApp\\MysteryCache\\old.bin').check();
    await expect(page.getByLabel('Typed confirmation')).toBeVisible();
    await page.getByLabel('Typed confirmation').fill('NOPE');
    await page.getByRole('button', { name: 'Preview Clean' }).click();
    await expect(page.getByRole('alert')).toContainText(/Typed confirmation required/i);

    await page.getByLabel('Typed confirmation').fill('DELETE');
    await page.getByRole('button', { name: 'Preview Clean' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
  });

  test('Settings and Logs navigation and a11y basics', async ({ page }) => {
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Primary' });
    await expect(nav).toBeVisible();
    await expect(page.getByRole('button', { name: 'Analyzer' })).toBeVisible();
    await expect(page.getByLabel('Tier 1 (system temp and caches)')).toBeVisible();
    await expect(page.getByLabel('Tier 2 (developer caches)')).toBeVisible();

    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    await expect(page.getByText('Minimum Age (hours)')).toBeVisible();

    await page.getByRole('button', { name: 'Logs' }).click();
    await expect(page.getByRole('heading', { name: 'Logs', exact: true })).toBeVisible();
    await expect(page.getByRole('table', { name: 'Log list' })).toBeVisible();

    await page.getByRole('button', { name: 'Scan & Preview' }).focus();
    await expect(page.getByRole('button', { name: 'Scan & Preview' })).toBeFocused();
  });

  test('Analyzer tree, selection rules, and Preview Selected -> Clean flow', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Analyzer' }).click();
    await expect(page.getByRole('heading', { name: 'Analyzer' })).toBeVisible();
    await expect(page.getByLabel('Analyzer root path')).toHaveValue('C:\\');

    await page.getByRole('button', { name: 'Analyze', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Analyzer Tree' })).toBeVisible();
    await expect(page.getByRole('tree', { name: 'Analyzer results tree' })).toBeVisible();

    await expect(page.getByRole('checkbox', { name: /dev-machine\.vhdx/ })).toBeDisabled();
    await expect(page.getByRole('checkbox', { name: /\\\.git/ })).toBeDisabled();

    await page.getByRole('button', { name: 'Expand Temp' }).click();
    await page.getByRole('checkbox', { name: /Temp\\a\.tmp/ }).check();
    await page.getByRole('button', { name: 'Preview Selected' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
    await expect(page.getByText(/Permanent deletion \(bypasses Recycle Bin\)/)).toBeVisible();

    await page.getByRole('button', { name: 'Clean Permanently' }).click();
    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
    await expect(page.getByText('Bytes freed')).toBeVisible();
  });
});
