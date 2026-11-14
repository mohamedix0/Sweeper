import { expect, test } from '@playwright/test';

type SweeperMock = {
  setElevationStatus: (status: { isElevated: boolean }) => void;
  setRelaunchResult: (result: { launched: boolean } | { error: { code: string; details: { reason?: string; count?: string } } }) => void;
  reset: () => void;
};

declare global {
  interface Window {
    __sweeperMock?: SweeperMock;
  }
}

test.describe('Sweeper Phase 5 (mock sandbox)', () => {
  test('shows non-elevated locked state and Run as administrator feedback', async ({ page }) => {
    await page.addInitScript(() => {
      (window as unknown as { __SWEEPER_MOCK_INIT__?: unknown }).__SWEEPER_MOCK_INIT__ = {
        relaunchResult: { error: { code: 'internal', details: { reason: 'Administrator launch was cancelled.' } } },
      };
    });
    await page.goto('/');
    await expect(page.getByLabel('Standard (Non-Elevated)')).toBeVisible();
    await expect(page.getByText(/Administrator-only Tier 3 targets locked/i)).toBeVisible();
    await page.getByRole('button', { name: 'Run as administrator' }).click();
    await expect(page.getByRole('alert')).toContainText(/Administrator launch was cancelled/i);
  });

  test('unlocks elevation targets settings flow when elevated', async ({ page }) => {
    await page.addInitScript(() => {
      (window as unknown as { __SWEEPER_MOCK_INIT__?: unknown }).__SWEEPER_MOCK_INIT__ = {
        elevationStatus: { isElevated: true },
      };
    });
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();
    await expect(page.getByLabel('Administrator (Elevated)')).toBeVisible();

    await page.getByLabel('Enable Tier 3 targets').check();
    const elevationToggle = page.getByLabel(
      'Allow Administrator-only Tier 3 targets (Windows Temp, Update cache, Prefetch, Package Cache)',
    );
    await expect(elevationToggle).toBeEnabled();
    await elevationToggle.check();
    await page.getByRole('button', { name: 'Save Settings' }).click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();
  });
  test('Advanced Settings: Tier 3 toggle, min-age warning, Custom Path add/reject/remove', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();

    const minAge = page.getByLabel('Minimum Age (hours)');
    await minAge.fill('0');
    await expect(page.getByText(/Minimum Age below 24 hours/i)).toBeVisible();

    const tier3 = page.getByLabel('Enable Tier 3 targets');
    await expect(tier3).not.toBeChecked();
    const staleDays = page.getByLabel('Stale project threshold (days)');
    await expect(staleDays).toBeDisabled();
    await tier3.check();
    await expect(staleDays).toBeEnabled();
    await staleDays.fill('45');
    await expect(page.getByText(/display-only/i)).toBeVisible();
    await expect(page.getByText(/Recycle Bin/i)).toBeVisible();

    await page.getByLabel('Add Custom Path').fill('C:\\Windows\\Temp\\bad');
    await page.getByRole('button', { name: 'Add & Validate' }).click();
    await expect(page.getByRole('alert')).toContainText(/Custom Path rejected/i);

    await page.getByLabel('Add Custom Path').fill('C:\\Users\\Alice\\CustomCleanup');
    await page.getByRole('button', { name: 'Add & Validate' }).click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();
    await expect(page.getByRole('table', { name: 'Custom Paths' })).toContainText('C:\\Users\\Alice\\CustomCleanup');

    await page.getByRole('button', { name: 'Remove' }).click();
    await expect(page.getByRole('table', { name: 'Custom Paths' })).toHaveCount(0);
  });

  test('Analyzer Unknown gating and typed confirmation end-to-end', async ({ page }) => {
    await page.goto('/');
    // Enable allow_delete_unknown
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByLabel('Allow deleting unknown folders (advanced)').check();
    await page.getByRole('button', { name: 'Save Settings' }).click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();

    await page.getByRole('button', { name: 'Analyzer' }).click();
    await page.getByRole('button', { name: 'Analyze', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Analyzer Tree' })).toBeVisible();

    const unknown = page.getByRole('checkbox', { name: /MysteryApp\\MysteryCache/ }).first();
    await expect(unknown).toBeEnabled();
    await unknown.check();

    await expect(page.getByLabel('Typed confirmation')).toBeVisible();
    await page.getByLabel('Typed confirmation').fill('delete');
    await expect(page.getByRole('button', { name: 'Preview Selected' })).toBeDisabled();
    await page.getByLabel('Typed confirmation').fill('DELETE');
    await page.getByRole('button', { name: 'Preview Selected' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
    await expect(page.getByText(/Permanent deletion \(bypasses Recycle Bin\)/)).toBeVisible();

    await page.getByRole('button', { name: 'Clean Permanently' }).click();
    await expect(page.getByRole('heading', { name: 'Clean Summary' })).toBeVisible();
  });

  test('Scan Custom Path/Unknown item requires typed confirmation when allowed', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.getByLabel('Allow deleting unknown folders (advanced)').check();
    await page.getByRole('button', { name: 'Save Settings' }).click();
    await expect(page.getByText(/Settings saved/i)).toBeVisible();

    await page.getByRole('button', { name: 'Scan & Preview' }).click();
    await page.getByRole('button', { name: 'Start Scan' }).click();
    await expect(page.getByRole('heading', { name: 'Scan Results' })).toBeVisible();

    await page.getByLabel('Select C:\\Users\\Alice\\CustomCleanup\\old.bin').check();
    await expect(page.getByLabel('Typed confirmation')).toBeVisible();
    await page.getByLabel('Typed confirmation').fill('DELETE');
    await page.getByRole('button', { name: 'Preview Clean' }).click();
    await expect(page.getByText('Preview Breakdown')).toBeVisible();
  });
});