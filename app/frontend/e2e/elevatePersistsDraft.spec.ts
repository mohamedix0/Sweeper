import { expect, test, type Page } from '@playwright/test';

type PersistedSettings = { minAgeHours: number };
type MockWithSettings = { getSettings: () => Promise<PersistedSettings> };

async function persistedMinAge(page: Page): Promise<number> {
  await page.waitForFunction(() => !!window.__sweeperMock);
  return page.evaluate(async () => {
    const mock = window.__sweeperMock as unknown as MockWithSettings;
    const settings = await mock.getSettings();
    return settings.minAgeHours;
  });
}

test.describe('Elevate with a pending Settings draft (mock sandbox)', () => {
  test('persists the draft before the restart is requested', async ({ page }) => {
    await page.goto('/');
    await page.getByRole('button', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();

    expect(await persistedMinAge(page)).toBe(24);

    const minAge = page.getByLabel('Minimum file age');
    await minAge.fill('12');
    await expect(minAge).toHaveValue('12');

    // Choosing Elevate from Settings must flush the pending draft through the
    // settings store before it asks the bridge to restart (#92).
    await page.getByLabel('Run as administrator').click();

    await expect.poll(() => persistedMinAge(page)).toBe(12);
    await expect(page.getByText(/Administrator launch requested/i)).toBeVisible();
  });
});
